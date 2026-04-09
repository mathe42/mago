use bumpalo::Bump;
use lsp_types::Hover;
use lsp_types::HoverContents;
use lsp_types::HoverParams;
use lsp_types::MarkupContent;
use lsp_types::MarkupKind;

use mago_codex::metadata::class_like::ClassLikeMetadata;
use mago_codex::metadata::function_like::FunctionLikeMetadata;
use mago_codex::ttype::TType;
use mago_names::resolver::NameResolver;
use mago_span::HasSpan;
use mago_syntax::ast::Statement;
use mago_syntax::comments::docblock::get_docblock_for_node;
use mago_syntax::parser::parse_file_content;

use crate::convert;
use crate::error::ServerError;
use crate::navigate;
use crate::navigate::SymbolAt;
use crate::state::LspState;

/// Handle `textDocument/hover`.
pub fn handle_hover(state: &LspState, params: HoverParams) -> Result<Option<Hover>, ServerError> {
    let uri = &params.text_document_position_params.text_document.uri;
    let position = params.text_document_position_params.position;

    let Some(file_id) = state.file_id_for_uri(uri) else {
        return Ok(None);
    };
    let Some(file) = state.get_file(&file_id) else {
        return Ok(None);
    };

    let offset = convert::lsp_position_to_offset(&file, position);

    let arena = Bump::new();
    let program = parse_file_content(&arena, file.id, &file.contents);
    let resolved_names = NameResolver::new(&arena).resolve(program);

    let codebase = state.codebase();
    let symbol = navigate::find_symbol_at_offset(program, &resolved_names, codebase, offset);

    let hover_range = symbol_span(&symbol).map(|span| convert::span_to_range(&file, span));

    let markdown = match symbol {
        SymbolAt::ClassLike { fqn, .. } => {
            let doc = find_class_docblock(program, &file, fqn, &resolved_names);
            codebase.get_class_like(fqn)
                .map(|meta| {
                    let mut s = format_class_like_hover(meta);
                    if let Some(d) = &doc {
                        let cleaned = clean_docblock(d);
                        if !cleaned.is_empty() {
                            s.push_str("\n\n");
                            s.push_str(&cleaned);
                        }
                    }
                    s
                })
                .or_else(|| {
                    let mut s = format!("```php\nclass {}\n```", fqn);
                    if let Some(d) = &doc {
                        let cleaned = clean_docblock(d);
                        if !cleaned.is_empty() {
                            s.push_str("\n\n");
                            s.push_str(&cleaned);
                        }
                    }
                    Some(s)
                })
        }
        SymbolAt::Function { fqn, .. } => {
            let raw_docblock = find_function_docblock(program, &file, fqn);
            codebase.get_function(fqn)
                .map(|meta| format_function_hover(meta, raw_docblock.as_deref()))
                .or_else(|| format_function_hover_from_ast(program, &file, fqn, offset))
        }
        SymbolAt::Method { ref class_fqn, ref method_name, .. } if !class_fqn.is_empty() => {
            let raw_docblock = find_method_docblock(program, &file, class_fqn, method_name, &resolved_names);
            codebase.get_declaring_method(class_fqn, method_name)
                .map(|meta| format_function_hover(meta, raw_docblock.as_deref()))
                .or_else(|| format_method_hover_from_ast(program, &file, class_fqn, method_name, &resolved_names))
        }
        SymbolAt::Method { ref method_name, .. } => {
            Some(format!("```php\n{}()\n```", method_name))
        }
        SymbolAt::Property { ref class_fqn, ref property_name, .. } if !class_fqn.is_empty() => {
            let lookup_name = if property_name.starts_with('$') {
                property_name.clone()
            } else {
                format!("${}", property_name)
            };
            let prop_doc = find_property_docblock(program, &file, class_fqn, &lookup_name, &resolved_names);
            let type_str = codebase.get_property_type(class_fqn, &lookup_name)
                .map(|t| t.get_id().to_string());
            let mut parts = Vec::new();
            if let Some(t) = &type_str {
                parts.push(format!("```php\n{}: {}\n```", lookup_name, t));
            } else {
                parts.push(format!("```php\n{}::{}\n```", class_fqn, lookup_name));
            }
            if let Some(d) = &prop_doc {
                let cleaned = clean_docblock(d);
                if !cleaned.is_empty() {
                    parts.push(cleaned);
                }
            }
            Some(parts.join("\n\n"))
        }
        SymbolAt::Property { ref property_name, .. } => {
            Some(format!("```php\n${}\n```", property_name))
        }
        SymbolAt::ClassConstant { ref class_fqn, ref constant_name, .. } => {
            codebase.get_class_constant(class_fqn, constant_name).map(|_meta| {
                format!("```php\n{}::{}\n```", class_fqn, constant_name)
            })
        }
        SymbolAt::Variable { ref name, .. } => {
            let raw_docblock = find_raw_docblock_at_offset(program, &file, offset);
            Some(format_variable_hover(name, raw_docblock.as_deref(), program, &file, &resolved_names, codebase, offset))
        }
        SymbolAt::Unknown => None,
    };

    let Some(content) = markdown else {
        return Ok(None);
    };

    Ok(Some(Hover {
        contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value: content }),
        range: hover_range,
    }))
}

fn symbol_span(symbol: &SymbolAt<'_>) -> Option<mago_span::Span> {
    match symbol {
        SymbolAt::ClassLike { span, .. }
        | SymbolAt::Function { span, .. }
        | SymbolAt::Method { span, .. }
        | SymbolAt::Property { span, .. }
        | SymbolAt::ClassConstant { span, .. }
        | SymbolAt::Variable { span, .. } => Some(*span),
        SymbolAt::Unknown => None,
    }
}

fn format_class_like_hover(meta: &ClassLikeMetadata) -> String {
    let mut parts = Vec::new();

    let kind = format!("{:?}", meta.kind).to_lowercase();
    let name = &meta.original_name;

    let mut signature = format!("{kind} {name}");

    if let Some(parent) = &meta.direct_parent_class {
        signature.push_str(&format!(" extends {parent}"));
    }

    if !meta.direct_parent_interfaces.is_empty() {
        let ifaces: Vec<&str> = meta.direct_parent_interfaces.iter().map(|a| a.as_str()).collect();
        let keyword = if kind == "interface" { "extends" } else { "implements" };
        signature.push_str(&format!(" {keyword} {}", ifaces.join(", ")));
    }

    parts.push(format!("```php\n{signature}\n```"));

    if !meta.methods.is_empty() {
        let count = meta.methods.len();
        parts.push(format!("{count} method(s)"));
    }

    parts.join("\n\n")
}

/// Build function hover from AST when codebase metadata is not available.
fn format_function_hover_from_ast(
    program: &mago_syntax::ast::Program<'_>,
    file: &mago_database::file::File,
    name: &str,
    offset: u32,
) -> Option<String> {
    for statement in program.statements.iter() {
        if let Statement::Function(func) = statement {
            if func.name.value == name || func.span().has_offset(offset) {
                let mut parts = Vec::new();

                // Build signature from source
                let sig_start = func.function.span().start.offset as usize;
                let sig_end = func.parameter_list.right_parenthesis.end_offset() as usize;
                let mut sig = if sig_end <= file.contents.len() {
                    file.contents[sig_start..sig_end].to_string()
                } else {
                    format!("function {}()", name)
                };

                // Return type hint
                if let Some(rth) = &func.return_type_hint {
                    let rth_start = rth.colon.start.offset as usize;
                    let rth_end = rth.hint.span().end_offset() as usize;
                    if rth_end <= file.contents.len() {
                        sig.push_str(&file.contents[rth_start..rth_end]);
                    }
                }

                parts.push(format!("```php\n{}\n```", sig));

                // Docblock
                if let Some(trivia) = get_docblock_for_node(program, file, func) {
                    let desc = clean_docblock(trivia.value);
                    if !desc.is_empty() {
                        parts.push(desc);
                    }
                    // @param descriptions
                    for param in func.parameter_list.parameters.iter() {
                        let param_name = format!("${}", param.variable.name);
                        if let Some(pdesc) = extract_param_doc(trivia.value, &param_name) {
                            parts.push(format!("*@param* `{}` — {}", param_name, pdesc));
                        }
                    }
                }

                return Some(parts.join("\n\n"));
            }
        }
    }
    None
}

/// Build method hover from AST when codebase metadata is not available.
fn format_method_hover_from_ast<'arena>(
    program: &'arena mago_syntax::ast::Program<'arena>,
    file: &mago_database::file::File,
    class_fqn: &str,
    method_name: &str,
    resolved_names: &mago_names::ResolvedNames<'arena>,
) -> Option<String> {
    format_method_hover_from_ast_recursive(program, file, class_fqn, method_name, resolved_names, &mut Vec::new())
}

fn format_method_hover_from_ast_recursive<'arena>(
    program: &'arena mago_syntax::ast::Program<'arena>,
    file: &mago_database::file::File,
    class_fqn: &str,
    method_name: &str,
    resolved_names: &mago_names::ResolvedNames<'arena>,
    visited: &mut Vec<String>,
) -> Option<String> {
    if visited.contains(&class_fqn.to_string()) {
        return None;
    }
    visited.push(class_fqn.to_string());

    for stmt in program.statements.iter() {
        let (fqn, members, parent, trait_uses) = match stmt {
            Statement::Class(c) => {
                let fqn = resolved_names.resolve(&c.name).unwrap_or(c.name.value);
                let parent = c.extends.as_ref().and_then(|ext| {
                    ext.types.iter().next().and_then(|t| resolved_names.resolve(t))
                });
                let traits: Vec<&str> = c.members.iter().filter_map(|m| {
                    if let mago_syntax::ast::ast::ClassLikeMember::TraitUse(tu) = m {
                        Some(tu.trait_names.iter().filter_map(|n| resolved_names.resolve(n)).collect::<Vec<_>>())
                    } else {
                        None
                    }
                }).flatten().collect();
                (fqn, &c.members, parent, traits)
            }
            Statement::Trait(t) => {
                let fqn = resolved_names.resolve(&t.name).unwrap_or(t.name.value);
                (fqn, &t.members, None, Vec::new())
            }
            _ => continue,
        };
        if fqn != class_fqn {
            continue;
        }
        // Check own methods
        for member in members.iter() {
            if let mago_syntax::ast::ast::ClassLikeMember::Method(method) = member {
                if method.name.value != method_name {
                    continue;
                }
                let mut parts = Vec::new();
                let sig_start = method.function.span().start.offset as usize;
                let sig_end = method.parameter_list.right_parenthesis.end_offset() as usize;
                let mut sig = if sig_end <= file.contents.len() {
                    file.contents[sig_start..sig_end].to_string()
                } else {
                    format!("function {}()", method_name)
                };
                if let Some(rth) = &method.return_type_hint {
                    let s = rth.colon.start.offset as usize;
                    let e = rth.hint.span().end_offset() as usize;
                    if e <= file.contents.len() {
                        sig.push_str(&file.contents[s..e]);
                    }
                }
                parts.push(format!("```php\n{}\n```", sig));
                if let Some(trivia) = get_docblock_for_node(program, file, method) {
                    let desc = clean_docblock(trivia.value);
                    if !desc.is_empty() {
                        parts.push(desc);
                    }
                    for param in method.parameter_list.parameters.iter() {
                        let pname = format!("${}", param.variable.name);
                        if let Some(pdesc) = extract_param_doc(trivia.value, &pname) {
                            parts.push(format!("*@param* `{}` — {}", pname, pdesc));
                        }
                    }
                }
                return Some(parts.join("\n\n"));
            }
        }
        // Search in traits
        for trait_fqn in &trait_uses {
            if let Some(result) = format_method_hover_from_ast_recursive(program, file, trait_fqn, method_name, resolved_names, visited) {
                return Some(result);
            }
        }
        // Search in parent
        if let Some(parent_fqn) = parent {
            if let Some(result) = format_method_hover_from_ast_recursive(program, file, parent_fqn, method_name, resolved_names, visited) {
                return Some(result);
            }
        }
    }
    None
}

/// Find the raw docblock string for the enclosing function/class at the given offset.
fn find_raw_docblock_at_offset<'arena>(
    program: &'arena mago_syntax::ast::Program<'arena>,
    file: &mago_database::file::File,
    offset: u32,
) -> Option<String> {
    for statement in program.statements.iter() {
        if !statement.span().has_offset(offset) {
            continue;
        }
        match statement {
            Statement::Function(func) => {
                if let Some(trivia) = get_docblock_for_node(program, file, func) {
                    return Some(trivia.value.to_string());
                }
            }
            Statement::Namespace(ns) => {
                let stmts = match &ns.body {
                    mago_syntax::ast::ast::NamespaceBody::Implicit(body) => &body.statements,
                    mago_syntax::ast::ast::NamespaceBody::BraceDelimited(block) => &block.statements,
                };
                for stmt in stmts.iter() {
                    if stmt.span().has_offset(offset) {
                        if let Statement::Function(func) = stmt {
                            if let Some(trivia) = get_docblock_for_node(program, file, func) {
                                return Some(trivia.value.to_string());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Find the raw docblock for a function by name (not by cursor offset).
fn find_function_docblock<'arena>(
    program: &'arena mago_syntax::ast::Program<'arena>,
    file: &mago_database::file::File,
    func_name: &str,
) -> Option<String> {
    for stmt in program.statements.iter() {
        if let Statement::Function(func) = stmt {
            if func.name.value == func_name {
                if let Some(trivia) = get_docblock_for_node(program, file, func) {
                    return Some(trivia.value.to_string());
                }
            }
        }
    }
    None
}

/// Find the raw docblock for a method by class/trait name, searching through
/// the class itself, its used traits, and parent classes.
fn find_method_docblock<'arena>(
    program: &'arena mago_syntax::ast::Program<'arena>,
    file: &mago_database::file::File,
    class_fqn: &str,
    method_name: &str,
    resolved_names: &mago_names::ResolvedNames<'arena>,
) -> Option<String> {
    find_method_docblock_recursive(program, file, class_fqn, method_name, resolved_names, &mut Vec::new())
}

fn find_method_docblock_recursive<'arena>(
    program: &'arena mago_syntax::ast::Program<'arena>,
    file: &mago_database::file::File,
    class_fqn: &str,
    method_name: &str,
    resolved_names: &mago_names::ResolvedNames<'arena>,
    visited: &mut Vec<String>,
) -> Option<String> {
    if visited.contains(&class_fqn.to_string()) {
        return None;
    }
    visited.push(class_fqn.to_string());

    for stmt in program.statements.iter() {
        let (fqn, members, parent, trait_uses) = match stmt {
            Statement::Class(c) => {
                let fqn = resolved_names.resolve(&c.name).unwrap_or(c.name.value);
                let parent = c.extends.as_ref().and_then(|ext| {
                    ext.types.iter().next().and_then(|t| resolved_names.resolve(t))
                });
                let traits: Vec<&str> = c.members.iter().filter_map(|m| {
                    if let mago_syntax::ast::ast::ClassLikeMember::TraitUse(tu) = m {
                        Some(tu.trait_names.iter().filter_map(|n| resolved_names.resolve(n)).collect::<Vec<_>>())
                    } else {
                        None
                    }
                }).flatten().collect();
                (fqn, &c.members, parent, traits)
            }
            Statement::Trait(t) => {
                let fqn = resolved_names.resolve(&t.name).unwrap_or(t.name.value);
                (fqn, &t.members, None, Vec::new())
            }
            _ => continue,
        };
        if fqn != class_fqn {
            continue;
        }
        // Check own methods
        for member in members.iter() {
            if let mago_syntax::ast::ast::ClassLikeMember::Method(method) = member {
                if method.name.value == method_name {
                    if let Some(trivia) = get_docblock_for_node(program, file, method) {
                        return Some(trivia.value.to_string());
                    }
                }
            }
        }
        // Check used traits
        for trait_fqn in &trait_uses {
            if let Some(doc) = find_method_docblock_recursive(program, file, trait_fqn, method_name, resolved_names, visited) {
                return Some(doc);
            }
        }
        // Check parent class
        if let Some(parent_fqn) = parent {
            if let Some(doc) = find_method_docblock_recursive(program, file, parent_fqn, method_name, resolved_names, visited) {
                return Some(doc);
            }
        }
    }
    None
}

/// Find the raw docblock for a property by class and property name.
fn find_property_docblock<'arena>(
    program: &'arena mago_syntax::ast::Program<'arena>,
    file: &mago_database::file::File,
    class_fqn: &str,
    property_name: &str, // with $ prefix
    resolved_names: &mago_names::ResolvedNames<'arena>,
) -> Option<String> {
    let bare_name = property_name.strip_prefix('$').unwrap_or(property_name);
    for stmt in program.statements.iter() {
        let (fqn, members) = match stmt {
            Statement::Class(c) => (resolved_names.resolve(&c.name).unwrap_or(c.name.value), &c.members),
            Statement::Trait(t) => (resolved_names.resolve(&t.name).unwrap_or(t.name.value), &t.members),
            _ => continue,
        };
        if fqn != class_fqn {
            continue;
        }
        for member in members.iter() {
            if let mago_syntax::ast::ast::ClassLikeMember::Property(prop) = member {
                let vars = match prop {
                    mago_syntax::ast::ast::class_like::property::Property::Plain(p) => {
                        p.items.iter().map(|i| i.variable()).collect::<Vec<_>>()
                    }
                    mago_syntax::ast::ast::class_like::property::Property::Hooked(h) => {
                        vec![h.item.variable()]
                    }
                };
                for var in &vars {
                    if var.name == bare_name || var.name == property_name {
                        if let Some(trivia) = get_docblock_for_node(program, file, prop) {
                            return Some(trivia.value.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Find the raw docblock for a class by FQN.
fn find_class_docblock<'arena>(
    program: &'arena mago_syntax::ast::Program<'arena>,
    file: &mago_database::file::File,
    class_fqn: &str,
    resolved_names: &mago_names::ResolvedNames<'arena>,
) -> Option<String> {
    for stmt in program.statements.iter() {
        match stmt {
            Statement::Class(c) => {
                let fqn = resolved_names.resolve(&c.name).unwrap_or(c.name.value);
                if fqn == class_fqn {
                    if let Some(trivia) = get_docblock_for_node(program, file, c) {
                        return Some(trivia.value.to_string());
                    }
                }
            }
            Statement::Trait(t) => {
                let fqn = resolved_names.resolve(&t.name).unwrap_or(t.name.value);
                if fqn == class_fqn {
                    if let Some(trivia) = get_docblock_for_node(program, file, t) {
                        return Some(trivia.value.to_string());
                    }
                }
            }
            Statement::Interface(i) => {
                let fqn = resolved_names.resolve(&i.name).unwrap_or(i.name.value);
                if fqn == class_fqn {
                    if let Some(trivia) = get_docblock_for_node(program, file, i) {
                        return Some(trivia.value.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Format hover for a variable — show type from parameter list, assignment RHS, or @param doc.
fn format_variable_hover<'arena>(
    var_name: &str,
    raw_docblock: Option<&str>,
    program: &'arena mago_syntax::ast::Program<'arena>,
    file: &mago_database::file::File,
    resolved_names: &mago_names::ResolvedNames<'arena>,
    codebase: &mago_codex::metadata::CodebaseMetadata,
    offset: u32,
) -> String {
    let mut parts = Vec::new();

    // 1. Try parameter type from enclosing function's AST.
    let param_type = find_parameter_type_hint(program, file, var_name, offset);

    // 2. Try resolving from assignment RHS: $x = new Foo() or $x = $a->method()
    let resolved_type = if param_type.is_none() {
        let normalized = if var_name.starts_with('$') { var_name.to_string() } else { format!("${}", var_name) };
        resolve_variable_type_from_assignments(program, &normalized, resolved_names, codebase)
    } else {
        None
    };

    let type_str = param_type.as_deref().or(resolved_type.as_deref());
    if let Some(t) = type_str {
        parts.push(format!("```php\n{} {}\n```", t, var_name));
    } else {
        parts.push(format!("```php\n{}\n```", var_name));
    }

    // Extract @param description from docblock.
    if let Some(raw) = raw_docblock {
        if let Some(desc) = extract_param_doc(raw, var_name) {
            parts.push(desc);
        }
    }

    parts.join("\n\n")
}

/// Resolve the type of a variable by looking at its assignment RHS.
fn resolve_variable_type_from_assignments<'arena>(
    program: &'arena mago_syntax::ast::Program<'arena>,
    var_name: &str,
    resolved_names: &mago_names::ResolvedNames<'arena>,
    codebase: &mago_codex::metadata::CodebaseMetadata,
) -> Option<String> {
    use mago_syntax::ast::ast::variable::Variable as AstVar;
    for stmt in program.statements.iter() {
        if let Statement::Expression(expr_stmt) = stmt {
            if let mago_syntax::ast::ast::Expression::Assignment(assign) = &expr_stmt.expression {
                if let mago_syntax::ast::ast::Expression::Variable(AstVar::Direct(dv)) = &*assign.lhs {
                    let lhs = if dv.name.starts_with('$') { dv.name.to_string() } else { format!("${}", dv.name) };
                    if lhs == var_name {
                        return resolve_expression_type(&assign.rhs, program, resolved_names, codebase);
                    }
                }
            }
        }
    }
    None
}

/// Try to determine the type of an expression.
pub fn resolve_expression_type<'arena>(
    expr: &mago_syntax::ast::ast::Expression<'arena>,
    program: &'arena mago_syntax::ast::Program<'arena>,
    resolved_names: &mago_names::ResolvedNames<'arena>,
    codebase: &mago_codex::metadata::CodebaseMetadata,
) -> Option<String> {
    use mago_syntax::ast::ast::Expression;
    use mago_syntax::ast::ast::variable::Variable as AstVar;
    match expr {
        // new ClassName()
        Expression::Instantiation(inst) => {
            if let Expression::Identifier(ident) = &*inst.class {
                resolved_names.resolve(ident).map(|s| s.to_string())
            } else {
                None
            }
        }
        // $var->method() — resolve return type
        Expression::Call(mago_syntax::ast::ast::Call::Method(call)) => {
            if let mago_syntax::ast::ast::ClassLikeMemberSelector::Identifier(method_ident) = &call.method {
                // Resolve the object's class
                let obj_class = resolve_expression_type(&call.object, program, resolved_names, codebase)?;
                // Look up the method's return type
                if let Some(meta) = codebase.get_declaring_method(&obj_class, &method_ident.value.to_string()) {
                    if let Some(rt) = &meta.return_type_metadata {
                        return Some(rt.type_union.get_id().to_string());
                    }
                }
                // Fallback: check AST return type hint
                resolve_method_return_type_from_ast(program, &obj_class, &method_ident.value.to_string(), resolved_names)
            } else {
                None
            }
        }
        // $var->property — resolve property type
        Expression::Access(mago_syntax::ast::ast::Access::Property(access)) => {
            if let mago_syntax::ast::ast::ClassLikeMemberSelector::Identifier(prop_ident) = &access.property {
                let obj_class = resolve_expression_type(&access.object, program, resolved_names, codebase)?;
                let lookup = format!("${}", prop_ident.value);
                if let Some(t) = codebase.get_property_type(&obj_class, &lookup) {
                    return Some(t.get_id().to_string());
                }
            }
            None
        }
        // $variable — resolve recursively
        Expression::Variable(AstVar::Direct(dv)) => {
            let name = if dv.name.starts_with('$') { dv.name.to_string() } else { format!("${}", dv.name) };
            crate::navigate::resolve_variable_class(program.statements.iter(), &name, resolved_names)
        }
        _ => None,
    }
}

/// Find return type of a method from the AST (when codebase doesn't have it).
pub fn resolve_method_return_type_from_ast<'arena>(
    program: &'arena mago_syntax::ast::Program<'arena>,
    class_name: &str,
    method_name: &str,
    resolved_names: &mago_names::ResolvedNames<'arena>,
) -> Option<String> {
    for stmt in program.statements.iter() {
        let (fqn, members) = match stmt {
            Statement::Class(c) => (resolved_names.resolve(&c.name).unwrap_or(c.name.value), &c.members),
            Statement::Trait(t) => (resolved_names.resolve(&t.name).unwrap_or(t.name.value), &t.members),
            _ => continue,
        };
        if fqn != class_name {
            continue;
        }
        for member in members.iter() {
            if let mago_syntax::ast::ast::ClassLikeMember::Method(method) = member {
                if method.name.value == method_name {
                    if let Some(rth) = &method.return_type_hint {
                        if let mago_syntax::ast::ast::Hint::Identifier(ident) = &rth.hint {
                            return resolved_names.resolve(ident).map(|s| s.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Find the type hint string for a parameter variable from the AST.
fn find_parameter_type_hint(
    program: &mago_syntax::ast::Program<'_>,
    file: &mago_database::file::File,
    var_name: &str,
    offset: u32,
) -> Option<String> {
    for statement in program.statements.iter() {
        if !statement.span().has_offset(offset) {
            continue;
        }
        if let Statement::Function(func) = statement {
            for param in func.parameter_list.parameters.iter() {
                if param.variable.name == var_name {
                    if let Some(hint) = &param.hint {
                        // Extract the type hint text from source.
                        let start = hint.span().start.offset as usize;
                        let end = hint.span().end_offset() as usize;
                        if end <= file.contents.len() {
                            return Some(file.contents[start..end].to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Extract the @param description for a specific variable from a raw docblock.
pub fn extract_param_doc(raw: &str, var_name: &str) -> Option<String> {
    let normalized = if var_name.starts_with('$') {
        var_name.to_string()
    } else {
        format!("${}", var_name)
    };

    for line in raw.lines() {
        let trimmed = line.trim();
        let content = trimmed.strip_prefix("* ").or_else(|| trimmed.strip_prefix("*")).unwrap_or(trimmed);
        if let Some(rest) = content.strip_prefix("@param") {
            let rest = rest.trim_start();
            // Split into words: could be "@param type $name desc" or "@param $name desc"
            let words: Vec<&str> = rest.split_whitespace().collect();
            for (i, word) in words.iter().enumerate() {
                if *word == normalized {
                    let desc: String = words[i + 1..].join(" ");
                    if !desc.is_empty() {
                        return Some(desc);
                    }
                    return None;
                }
            }
        }
    }
    None
}

/// Strip the leading `/** `, trailing ` */`, and `*` prefixes from each line.
pub fn clean_docblock(raw: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        // Skip opening/closing markers
        if trimmed == "/**" || trimmed == "*/" {
            continue;
        }
        let content = trimmed.strip_prefix("* ").or_else(|| trimmed.strip_prefix("*")).unwrap_or(trimmed);
        // Skip @-tags (they are already represented in the signature)
        if content.starts_with('@') {
            continue;
        }
        if !content.is_empty() {
            lines.push(content);
        }
    }
    lines.join("\n")
}

fn format_function_hover(meta: &FunctionLikeMetadata, raw_docblock: Option<&str>) -> String {
    let mut parts = Vec::new();
    let name = meta.original_name.as_ref().map(|a| a.as_str()).unwrap_or("anonymous");

    let mut sig = String::new();

    // Visibility for methods
    if let Some(method_meta) = &meta.method_metadata {
        sig.push_str(&format!("{:?} ", method_meta.visibility).to_lowercase());
        if method_meta.is_static {
            sig.push_str("static ");
        }
        if method_meta.is_abstract {
            sig.push_str("abstract ");
        }
    }

    sig.push_str("function ");
    sig.push_str(name);
    sig.push('(');

    // Parameters
    let params: Vec<String> = meta
        .parameters
        .iter()
        .map(|p| {
            let mut param = String::new();
            if let Some(type_meta) = &p.type_metadata {
                param.push_str(&format!("{} ", type_meta.type_union.get_id()));
            }
            param.push_str(&p.name.0.to_string());
            param
        })
        .collect();
    sig.push_str(&params.join(", "));
    sig.push(')');

    // Return type
    if let Some(return_meta) = &meta.return_type_metadata {
        sig.push_str(&format!(": {}", return_meta.type_union.get_id()));
    }

    parts.push(format!("```php\n{sig}\n```"));

    if let Some(raw) = raw_docblock {
        let description = clean_docblock(raw);
        if !description.is_empty() {
            parts.push(description);
        }

        // Show @param descriptions
        let param_docs: Vec<String> = meta
            .parameters
            .iter()
            .filter_map(|p| {
                let param_name = &p.name.0.to_string();
                extract_param_doc(raw, param_name).map(|desc| format!("*@param* `{}` — {}", param_name, desc))
            })
            .collect();
        if !param_docs.is_empty() {
            parts.push(param_docs.join("\n\n"));
        }
    }

    parts.join("\n\n")
}
