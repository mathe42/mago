use bumpalo::Bump;
use lsp_types::CompletionItem;
use lsp_types::CompletionItemKind;
use lsp_types::CompletionItemLabelDetails;
use lsp_types::CompletionParams;
use lsp_types::InsertTextFormat;

use mago_codex::metadata::CodebaseMetadata;
use mago_codex::metadata::function_like::FunctionLikeMetadata;
use mago_codex::ttype::TType;
use mago_names::resolver::NameResolver;
use mago_span::HasSpan;
use mago_syntax::ast::Program;
use mago_syntax::ast::ast::ClassLikeMember;
use mago_syntax::ast::ast::Expression;
use mago_syntax::ast::ast::Statement;
use mago_syntax::parser::parse_file_content;

use crate::convert;
use crate::error::ServerError;
use crate::state::LspState;

/// The detected completion context based on text analysis.
enum CompletionContext {
    /// After `->` on an object. Contains the variable name (e.g. "$a") for type resolution.
    MemberAccess(Option<String>),
    /// After `::` on a class. Contains the FQCN.
    StaticAccess(String),
    /// Typing a variable name (after `$`).
    Variable,
    /// After `new ` keyword.
    NewExpression,
    /// In a type hint position or general identifier.
    TypeOrFunction,
    /// Unknown context.
    Unknown,
}

/// Handle `textDocument/completion`.
pub fn handle_completion(
    state: &LspState,
    params: CompletionParams,
) -> Result<Option<Vec<CompletionItem>>, ServerError> {
    let uri = &params.text_document_position.text_document.uri;
    let position = params.text_document_position.position;

    let Some(file_id) = state.file_id_for_uri(uri) else {
        return Ok(None);
    };
    let Some(file) = state.get_file(&file_id) else {
        return Ok(None);
    };

    let offset = convert::lsp_position_to_offset(&file, position);
    let source = &file.contents;

    // Check if cursor is inside an embedded language region (SQL/Bash).
    if let Some(embedded_items) = super::embedded::get_embedded_completions(&file, offset) {
        if !embedded_items.is_empty() {
            return Ok(Some(embedded_items));
        }
    }

    // Detect PHP context from the text before cursor.
    let context = detect_context(source, offset as usize);
    let codebase = state.codebase();

    let items = match context {
        CompletionContext::MemberAccess(var_name) => {
            let arena = Bump::new();
            let program = parse_file_content(&arena, file.id, &file.contents);
            let resolved_names = NameResolver::new(&arena).resolve(program);

            // First try simple variable resolution
            let class_fqn = match var_name.as_deref() {
                Some("$this") => find_this_class(program, &resolved_names, offset),
                Some(name) => {
                    crate::navigate::resolve_variable_class(
                        program.statements.iter(),
                        name,
                        &resolved_names,
                    ).or_else(|| find_this_class(program, &resolved_names, offset))
                }
                None => find_this_class(program, &resolved_names, offset),
            };

            // If simple resolution failed, try AST-based expression type resolution
            // This handles chains like $a->method()->
            let class_fqn = class_fqn.or_else(|| {
                find_object_type_at_offset(program, &resolved_names, codebase, offset)
            });

            if let Some(fqn) = class_fqn {
                complete_members(codebase, &fqn, false)
            } else {
                vec![]
            }
        }
        CompletionContext::StaticAccess(class_fqn) => {
            complete_members(codebase, &class_fqn, true)
        }
        CompletionContext::Variable => {
            let arena = Bump::new();
            let program = parse_file_content(&arena, file.id, &file.contents);
            complete_variables(program, offset)
        }
        CompletionContext::NewExpression | CompletionContext::TypeOrFunction => {
            let mut items = complete_class_names(codebase);
            // Also add classes/functions from the current file's AST (in case codebase is incomplete).
            let arena = Bump::new();
            let program = parse_file_content(&arena, file.id, &file.contents);
            let resolved_names = NameResolver::new(&arena).resolve(program);
            items.extend(complete_class_names_from_ast(program, &resolved_names));
            if matches!(context, CompletionContext::TypeOrFunction) {
                items.extend(complete_function_names(codebase));
                items.extend(complete_function_names_from_ast(program, &resolved_names, &file));
            }
            // Deduplicate by label
            let mut seen = std::collections::HashSet::new();
            items.retain(|item| seen.insert(item.label.clone()));
            items
        }
        CompletionContext::Unknown => {
            let mut items = complete_class_names(codebase);
            items.extend(complete_function_names(codebase));
            items
        }
    };

    Ok(Some(items))
}

/// Detect the completion context by examining text before the cursor.
fn detect_context(source: &str, offset: usize) -> CompletionContext {
    let before = &source[..offset.min(source.len())];
    let trimmed = before.trim_end();

    // Check for `->` (member access)
    if trimmed.ends_with("->") || ends_with_arrow_and_partial(trimmed) {
        // Extract the variable name before `->`
        let pre_arrow = trimmed.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
        let pre_arrow = pre_arrow.trim_end_matches("->");
        let pre_arrow = pre_arrow.trim_end();
        // Extract `$varname` from e.g. `... $this` or `... $a`
        if let Some(dollar_pos) = pre_arrow.rfind('$') {
            let var_name = &pre_arrow[dollar_pos..];
            if var_name.len() > 1 && var_name[1..].chars().all(|c| c.is_alphanumeric() || c == '_') {
                return CompletionContext::MemberAccess(Some(var_name.to_string()));
            }
        }
        return CompletionContext::MemberAccess(None);
    }

    // Check for `::` (static access)
    if trimmed.ends_with("::") || ends_with_double_colon_and_partial(trimmed) {
        // Extract the class name before `::`
        let pre_colon = trimmed.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
        let pre_colon = pre_colon.trim_end_matches("::");
        if let Some(class_name) = extract_identifier_before(pre_colon) {
            return CompletionContext::StaticAccess(class_name.to_string());
        }
        return CompletionContext::Unknown;
    }

    // Check for `$` (variable)
    if trimmed.ends_with('$') || (trimmed.len() > 0 && is_in_variable(trimmed)) {
        return CompletionContext::Variable;
    }

    // Check for `new ` keyword (with or without partial class name)
    if trimmed.ends_with("new ") || trimmed.ends_with("new\t") {
        return CompletionContext::NewExpression;
    }
    {
        // Match `new PartialName` — strip trailing identifier then check for `new `
        let before_ident = trimmed.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '\\');
        let before_ident = before_ident.trim_end();
        if before_ident.ends_with("new") {
            return CompletionContext::NewExpression;
        }
    }

    CompletionContext::TypeOrFunction
}

fn ends_with_arrow_and_partial(s: &str) -> bool {
    // Matches `->partial_identifier`
    let s = s.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
    s.ends_with("->")
}

fn ends_with_double_colon_and_partial(s: &str) -> bool {
    let s = s.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '$');
    s.ends_with("::")
}

fn is_in_variable(s: &str) -> bool {
    // Check if cursor is in the middle of typing a variable name: $par|
    let bytes = s.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        let c = bytes[i] as char;
        if c == '$' {
            return true;
        }
        if !c.is_alphanumeric() && c != '_' {
            return false;
        }
    }
    false
}

fn extract_identifier_before(s: &str) -> Option<&str> {
    let trimmed = s.trim_end();
    let start = trimmed
        .rfind(|c: char| !c.is_alphanumeric() && c != '_' && c != '\\')
        .map(|i| i + 1)
        .unwrap_or(0);
    let ident = &trimmed[start..];
    if ident.is_empty() { None } else { Some(ident) }
}

/// Complete class members (methods + properties).
fn complete_members(codebase: &CodebaseMetadata, class_fqn: &str, static_only: bool) -> Vec<CompletionItem> {
    let mut items = Vec::new();

    let Some(class_meta) = codebase.get_class_like(class_fqn) else {
        return items;
    };

    // Methods
    for method_name in class_meta.methods.iter() {
        let key = (mago_atom::ascii_lowercase_atom(class_fqn), method_name.clone());
        if let Some(method_meta) = codebase.function_likes.get(&key) {
            if static_only {
                if let Some(ref mm) = method_meta.method_metadata {
                    if !mm.is_static && !mm.is_constructor {
                        continue;
                    }
                }
            }

            let name = method_meta
                .original_name
                .as_ref()
                .map(|a| a.to_string())
                .unwrap_or_else(|| method_name.to_string());

            let detail = build_function_detail(method_meta);

            items.push(CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::METHOD),
                label_details: Some(CompletionItemLabelDetails {
                    detail: None,
                    description: Some(detail.clone()),
                }),
                detail: Some(detail),
                insert_text: Some(format!("{}($0)", name)),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            });
        }
    }

    // Properties
    for (prop_name, prop_meta) in class_meta.properties.iter() {
        let name_str = prop_name.as_str();

        // For instance access ($a->member), strip the leading '$'.
        // For static access (Class::$prop), keep it.
        let display_name = if !static_only {
            name_str.strip_prefix('$').unwrap_or(name_str)
        } else {
            name_str
        };

        let type_info = prop_meta
            .type_metadata
            .as_ref()
            .map(|tm| tm.type_union.get_id().to_string())
            .unwrap_or_default();

        items.push(CompletionItem {
            label: display_name.to_string(),
            kind: Some(CompletionItemKind::PROPERTY),
            label_details: if type_info.is_empty() {
                None
            } else {
                Some(CompletionItemLabelDetails {
                    detail: None,
                    description: Some(type_info.clone()),
                })
            },
            detail: if type_info.is_empty() { None } else { Some(type_info) },
            ..Default::default()
        });
    }

    // Constants (for static access)
    if static_only {
        for (const_name, _const_meta) in class_meta.constants.iter() {
            items.push(CompletionItem {
                label: const_name.to_string(),
                kind: Some(CompletionItemKind::CONSTANT),
                ..Default::default()
            });
        }

        // Enum cases
        for (case_name, _case_meta) in class_meta.enum_cases.iter() {
            items.push(CompletionItem {
                label: case_name.to_string(),
                kind: Some(CompletionItemKind::ENUM_MEMBER),
                ..Default::default()
            });
        }
    }

    items
}

fn build_function_detail(meta: &FunctionLikeMetadata) -> String {
    let mut detail = String::new();
    if let Some(mm) = &meta.method_metadata {
        detail.push_str(&format!("{:?} ", mm.visibility).to_lowercase());
    }
    detail.push('(');
    let params: Vec<String> = meta.parameters.iter().map(|p| {
        let mut s = String::new();
        if let Some(tm) = &p.type_metadata {
            s.push_str(&tm.type_union.get_id().to_string());
            s.push(' ');
        }
        s.push_str(&p.name.0.to_string());
        s
    }).collect();
    detail.push_str(&params.join(", "));
    detail.push(')');
    if let Some(rt) = &meta.return_type_metadata {
        detail.push_str(&format!(": {}", rt.type_union.get_id()));
    }
    detail
}

/// Complete variable names by scanning the AST.
fn complete_variables<'arena>(program: &Program<'arena>, offset: u32) -> Vec<CompletionItem> {
    let mut variables = std::collections::HashSet::new();

    // Walk statements to collect variables.
    for statement in program.statements.iter() {
        collect_variables_from_statement(statement, offset, &mut variables);
    }

    variables
        .into_iter()
        .map(|name| CompletionItem {
            label: name.clone(),
            kind: Some(CompletionItemKind::VARIABLE),
            ..Default::default()
        })
        .collect()
}

fn collect_variables_from_statement<'arena>(
    stmt: &Statement<'arena>,
    offset: u32,
    vars: &mut std::collections::HashSet<String>,
) {
    match stmt {
        Statement::Function(func) => {
            if func.span().has_offset(offset) {
                // Collect parameters
                for param in func.parameter_list.parameters.iter() {
                    vars.insert(format!("${}", param.variable.name));
                }
                // Collect variables from body
                for s in func.body.statements.iter() {
                    if s.span().start.offset < offset {
                        collect_variables_from_statement(s, offset, vars);
                    }
                }
            }
        }
        Statement::Class(class) => {
            for member in class.members.iter() {
                if member.span().has_offset(offset) {
                    if let ClassLikeMember::Method(method) = member {
                        vars.insert("$this".to_string());
                        for param in method.parameter_list.parameters.iter() {
                            vars.insert(format!("${}", param.variable.name));
                        }
                        if let mago_syntax::ast::ast::MethodBody::Concrete(block) = &method.body {
                            for s in block.statements.iter() {
                                if s.span().start.offset < offset {
                                    collect_variables_from_statement(s, offset, vars);
                                }
                            }
                        }
                    }
                }
            }
        }
        Statement::Expression(expr_stmt) => {
            collect_variables_from_expression(&expr_stmt.expression, vars);
        }
        Statement::Block(block) => {
            for s in block.statements.iter() {
                if s.span().start.offset < offset {
                    collect_variables_from_statement(s, offset, vars);
                }
            }
        }
        Statement::Namespace(ns) => {
            let stmts = match &ns.body {
                mago_syntax::ast::ast::NamespaceBody::Implicit(body) => &body.statements,
                mago_syntax::ast::ast::NamespaceBody::BraceDelimited(block) => &block.statements,
            };
            for s in stmts.iter() {
                collect_variables_from_statement(s, offset, vars);
            }
        }
        Statement::Foreach(foreach) => {
            // Collect the iteration variable
            if foreach.span().has_offset(offset) {
                collect_variables_from_foreach_target(&foreach.target, vars);
                if let mago_syntax::ast::ast::ForeachBody::Statement(stmt) = &foreach.body {
                    collect_variables_from_statement(stmt, offset, vars);
                } else if let mago_syntax::ast::ast::ForeachBody::ColonDelimited(body) = &foreach.body {
                    for s in body.statements.iter() {
                        collect_variables_from_statement(s, offset, vars);
                    }
                }
            }
        }
        Statement::If(if_stmt) => {
            collect_variables_from_expression(&if_stmt.condition, vars);
        }
        _ => {}
    }
}

fn collect_variables_from_foreach_target<'arena>(
    target: &mago_syntax::ast::ast::ForeachTarget<'arena>,
    vars: &mut std::collections::HashSet<String>,
) {
    match target {
        mago_syntax::ast::ast::ForeachTarget::Value(v) => {
            collect_variables_from_expression(&v.value, vars);
        }
        mago_syntax::ast::ast::ForeachTarget::KeyValue(kv) => {
            collect_variables_from_expression(&kv.key, vars);
            collect_variables_from_expression(&kv.value, vars);
        }
    }
}

fn collect_variables_from_expression<'arena>(
    expr: &Expression<'arena>,
    vars: &mut std::collections::HashSet<String>,
) {
    match expr {
        Expression::Variable(mago_syntax::ast::ast::Variable::Direct(dv)) => {
            vars.insert(format!("${}", dv.name));
        }
        Expression::Assignment(assign) => {
            collect_variables_from_expression(&*assign.lhs, vars);
            collect_variables_from_expression(&*assign.rhs, vars);
        }
        _ => {}
    }
}

/// Complete class/interface/trait/enum names.
fn complete_class_names(codebase: &CodebaseMetadata) -> Vec<CompletionItem> {
    codebase
        .class_likes
        .iter()
        .take(200) // Limit results
        .map(|(fqn, meta)| {
            let kind = match meta.kind {
                mago_codex::symbol::SymbolKind::Interface => CompletionItemKind::INTERFACE,
                mago_codex::symbol::SymbolKind::Enum => CompletionItemKind::ENUM,
                mago_codex::symbol::SymbolKind::Trait => CompletionItemKind::INTERFACE,
                _ => CompletionItemKind::CLASS,
            };
            let kind_str = format!("{:?}", meta.kind).to_lowercase();
            let detail = if fqn.as_str() != meta.original_name.as_str() {
                format!("{} ({})", kind_str, fqn)
            } else {
                kind_str
            };
            CompletionItem {
                label: meta.original_name.to_string(),
                kind: Some(kind),
                label_details: Some(CompletionItemLabelDetails {
                    detail: None,
                    description: Some(detail.clone()),
                }),
                detail: Some(detail),
                ..Default::default()
            }
        })
        .collect()
}

/// Complete global function names.
fn complete_function_names(codebase: &CodebaseMetadata) -> Vec<CompletionItem> {
    codebase
        .function_likes
        .iter()
        .filter(|((scope, _), _)| scope.is_empty()) // Only global functions
        .take(200)
        .filter_map(|((_, name), meta)| {
            let display_name = meta
                .original_name
                .as_ref()
                .map(|a| a.to_string())
                .unwrap_or_else(|| name.to_string());

            let detail = build_function_detail(meta);
            Some(CompletionItem {
                label: display_name.clone(),
                kind: Some(CompletionItemKind::FUNCTION),
                label_details: Some(CompletionItemLabelDetails {
                    detail: None,
                    description: Some(detail.clone()),
                }),
                detail: Some(detail),
                insert_text: Some(format!("{}($0)", display_name)),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            })
        })
        .collect()
}

/// Complete class names from the current file's AST.
fn complete_class_names_from_ast<'arena>(
    program: &Program<'arena>,
    resolved_names: &mago_names::ResolvedNames<'arena>,
) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    for stmt in program.statements.iter() {
        match stmt {
            Statement::Class(class) => {
                let name = resolved_names.resolve(&class.name)
                    .unwrap_or(class.name.value);
                items.push(CompletionItem {
                    label: name.to_string(),
                    kind: Some(CompletionItemKind::CLASS),
                    detail: Some("class".to_string()),
                    ..Default::default()
                });
            }
            Statement::Interface(iface) => {
                let name = resolved_names.resolve(&iface.name)
                    .unwrap_or(iface.name.value);
                items.push(CompletionItem {
                    label: name.to_string(),
                    kind: Some(CompletionItemKind::INTERFACE),
                    detail: Some("interface".to_string()),
                    ..Default::default()
                });
            }
            Statement::Enum(e) => {
                let name = resolved_names.resolve(&e.name)
                    .unwrap_or(e.name.value);
                items.push(CompletionItem {
                    label: name.to_string(),
                    kind: Some(CompletionItemKind::ENUM),
                    detail: Some("enum".to_string()),
                    ..Default::default()
                });
            }
            Statement::Trait(t) => {
                let name = resolved_names.resolve(&t.name)
                    .unwrap_or(t.name.value);
                items.push(CompletionItem {
                    label: name.to_string(),
                    kind: Some(CompletionItemKind::INTERFACE),
                    detail: Some("trait".to_string()),
                    ..Default::default()
                });
            }
            _ => {}
        }
    }
    items
}

/// Complete function names from the current file's AST.
fn complete_function_names_from_ast<'arena>(
    program: &Program<'arena>,
    resolved_names: &mago_names::ResolvedNames<'arena>,
    file: &mago_database::file::File,
) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    for stmt in program.statements.iter() {
        if let Statement::Function(func) = stmt {
            let name = resolved_names.resolve(&func.name)
                .unwrap_or(func.name.value);
            // Build detail from source
            let params: Vec<String> = func.parameter_list.parameters.iter().map(|p| {
                let start = p.span().start.offset as usize;
                let end = p.span().end_offset() as usize;
                if end <= file.contents.len() {
                    file.contents[start..end].to_string()
                } else {
                    format!("${}", p.variable.name)
                }
            }).collect();
            let mut detail = format!("({})", params.join(", "));
            if let Some(rth) = &func.return_type_hint {
                let start = rth.colon.start.offset as usize;
                let end = rth.hint.span().end_offset() as usize;
                if end <= file.contents.len() {
                    detail.push_str(&file.contents[start..end]);
                }
            }
            items.push(CompletionItem {
                label: name.to_string(),
                kind: Some(CompletionItemKind::FUNCTION),
                label_details: Some(CompletionItemLabelDetails {
                    detail: None,
                    description: Some(detail.clone()),
                }),
                detail: Some(detail),
                insert_text: Some(format!("{}($0)", name)),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            });
        }
    }
    items
}

/// Resolve the type of the object expression before `->` at the given offset.
/// Handles chains like `$a->method()->` by walking the AST.
fn find_object_type_at_offset<'arena>(
    program: &Program<'arena>,
    resolved_names: &mago_names::ResolvedNames<'arena>,
    codebase: &mago_codex::metadata::CodebaseMetadata,
    offset: u32,
) -> Option<String> {
    for stmt in program.statements.iter() {
        if !stmt.span().has_offset(offset) {
            continue;
        }
        if let Some(ty) = find_object_type_in_stmt(stmt, program, resolved_names, codebase, offset) {
            return Some(ty);
        }
    }
    None
}

fn find_object_type_in_stmt<'arena>(
    stmt: &Statement<'arena>,
    program: &'arena Program<'arena>,
    resolved_names: &mago_names::ResolvedNames<'arena>,
    codebase: &mago_codex::metadata::CodebaseMetadata,
    offset: u32,
) -> Option<String> {
    match stmt {
        Statement::Expression(expr_stmt) => find_object_type_in_expr(&expr_stmt.expression, program, resolved_names, codebase, offset),
        Statement::Echo(echo) => {
            for val in echo.values.iter() {
                if let Some(ty) = find_object_type_in_expr(val, program, resolved_names, codebase, offset) {
                    return Some(ty);
                }
            }
            None
        }
        Statement::Return(ret) => ret.value.as_ref().and_then(|v| find_object_type_in_expr(v, program, resolved_names, codebase, offset)),
        _ => None,
    }
}

fn find_object_type_in_expr<'arena>(
    expr: &mago_syntax::ast::ast::Expression<'arena>,
    program: &'arena Program<'arena>,
    resolved_names: &mago_names::ResolvedNames<'arena>,
    codebase: &mago_codex::metadata::CodebaseMetadata,
    offset: u32,
) -> Option<String> {
    use mago_syntax::ast::ast::Expression;
    if !expr.span().has_offset(offset) {
        return None;
    }
    match expr {
        // $obj->method() — if cursor is AFTER this call (in a chained ->), resolve the call's type
        Expression::Call(mago_syntax::ast::ast::Call::Method(call)) => {
            // Check if cursor is in a further chained access — we need the result type of THIS call
            // First recurse into object
            if call.object.span().has_offset(offset) {
                return find_object_type_in_expr(&call.object, program, resolved_names, codebase, offset);
            }
            // The cursor is on/after the method name — resolve the object's type
            super::hover::resolve_expression_type(&call.object, program, resolved_names, codebase)
        }
        Expression::Access(mago_syntax::ast::ast::Access::Property(access)) => {
            if access.object.span().has_offset(offset) {
                return find_object_type_in_expr(&access.object, program, resolved_names, codebase, offset);
            }
            super::hover::resolve_expression_type(&access.object, program, resolved_names, codebase)
        }
        Expression::Assignment(assign) => {
            find_object_type_in_expr(&assign.lhs, program, resolved_names, codebase, offset)
                .or_else(|| find_object_type_in_expr(&assign.rhs, program, resolved_names, codebase, offset))
        }
        _ => None,
    }
}

/// Find the enclosing class FQCN when `$this->` is used.
fn find_this_class<'arena>(
    program: &Program<'arena>,
    resolved_names: &mago_names::ResolvedNames<'arena>,
    offset: u32,
) -> Option<String> {
    for stmt in program.statements.iter() {
        if let Some(fqn) = find_class_scope(stmt, resolved_names, offset) {
            return Some(fqn);
        }
    }
    None
}

fn find_class_scope<'arena>(
    stmt: &Statement<'arena>,
    resolved_names: &mago_names::ResolvedNames<'arena>,
    offset: u32,
) -> Option<String> {
    match stmt {
        Statement::Class(class) if class.span().has_offset(offset) => {
            resolved_names.resolve(&class.name).map(|s| s.to_string())
        }
        Statement::Namespace(ns) => {
            let stmts = match &ns.body {
                mago_syntax::ast::ast::NamespaceBody::Implicit(body) => &body.statements,
                mago_syntax::ast::ast::NamespaceBody::BraceDelimited(block) => &block.statements,
            };
            for s in stmts.iter() {
                if let Some(fqn) = find_class_scope(s, resolved_names, offset) {
                    return Some(fqn);
                }
            }
            None
        }
        _ => None,
    }
}
