use bumpalo::Bump;
use lsp_types::ParameterInformation;
use lsp_types::ParameterLabel;
use lsp_types::SignatureHelp;
use lsp_types::SignatureHelpParams;
use lsp_types::SignatureInformation;

use mago_codex::metadata::function_like::FunctionLikeMetadata;
use mago_codex::ttype::TType;
use mago_database::file::File;
use mago_names::resolver::NameResolver;
use mago_syntax::ast::Program;
use mago_syntax::comments::docblock::get_docblock_for_node;
use mago_syntax::parser::parse_file_content;

use crate::convert;
use crate::error::ServerError;
use crate::state::LspState;

/// Handle `textDocument/signatureHelp`.
pub fn handle_signature_help(
    state: &LspState,
    params: SignatureHelpParams,
) -> Result<Option<SignatureHelp>, ServerError> {
    let uri = &params.text_document_position_params.text_document.uri;
    let position = params.text_document_position_params.position;

    let Some(file_id) = state.file_id_for_uri(uri) else {
        return Ok(None);
    };
    let Some(file) = state.get_file(&file_id) else {
        return Ok(None);
    };

    let byte_offset = convert::lsp_position_to_offset(&file, position);

    // Check if cursor is inside an embedded SQL region first.
    if let Some(sig_help) = super::embedded::get_embedded_signature_help(&file, byte_offset) {
        return Ok(Some(sig_help));
    }

    let offset = byte_offset as usize;
    let source = &file.contents;

    // Find the function name and active parameter index from text before cursor.
    let Some((func_name, active_param)) = find_call_context(source, offset) else {
        return Ok(None);
    };

    // Parse and resolve the function name.
    let arena = Bump::new();
    let program = parse_file_content(&arena, file.id, &file.contents);
    let resolved_names = NameResolver::new(&arena).resolve(program);
    let codebase = state.codebase();

    // Detect if this is a constructor call: `new ClassName(`
    let is_constructor = {
        let before = &source[..offset.min(source.len())];
        let trimmed = before.trim_end();
        // Walk back past the partial args and opening paren to check for `new`
        let before_call = trimmed.rfind('(').map(|i| &trimmed[..i]).unwrap_or(trimmed);
        let before_name = before_call.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '\\').trim_end();
        before_name.ends_with("new")
    };

    // Detect if this is a method call: `$var->method(`
    let method_context = {
        let before = &source[..offset.min(source.len())];
        let trimmed = before.trim_end();
        if let Some(paren_pos) = trimmed.rfind('(') {
            let before_paren = &trimmed[..paren_pos];
            let method_name_start = before_paren.rfind("->").map(|i| i + 2);
            if let Some(start) = method_name_start {
                let method = &before_paren[start..];
                if !method.is_empty() && method.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    // Extract the variable before ->
                    let before_arrow = &before_paren[..start - 2].trim_end();
                    if let Some(dollar) = before_arrow.rfind('$') {
                        let var = &before_arrow[dollar..];
                        if var.len() > 1 && var[1..].chars().all(|c| c.is_alphanumeric() || c == '_') {
                            Some((var.to_string(), method.to_string()))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };

    // Resolve function/method metadata
    let meta = if is_constructor {
        // Look up __construct on the class
        let class_fqn = resolve_class_name(&func_name, &resolved_names, program);
        class_fqn.and_then(|fqn| codebase.get_declaring_method(&fqn, "__construct"))
    } else if let Some((var_name, method_name)) = &method_context {
        // Resolve variable type, then look up method
        let class_fqn = crate::navigate::resolve_variable_class(
            program.statements.iter(), var_name, &resolved_names,
        );
        class_fqn.and_then(|fqn| codebase.get_declaring_method(&fqn, method_name))
    } else {
        resolve_function_meta(&func_name, &resolved_names, codebase, program)
    };

    let Some(meta) = meta else {
        // Fallback: try AST-based signature for functions/methods
        let sig = if is_constructor {
            build_constructor_signature_from_ast(program, &func_name, &resolved_names, &file)
        } else if let Some((var_name, method_name)) = &method_context {
            let class_fqn = crate::navigate::resolve_variable_class(
                program.statements.iter(), var_name, &resolved_names,
            );
            class_fqn.and_then(|fqn| build_method_signature_from_ast(program, &fqn, method_name, &resolved_names, &file))
        } else {
            build_function_signature_from_ast(program, &func_name, &file)
        };
        return Ok(sig.map(|s| SignatureHelp {
            signatures: vec![s],
            active_signature: Some(0),
            active_parameter: Some(active_param),
        }));
    };

    let docblock = find_function_docblock(program, &file, &func_name);
    let sig = build_signature(meta, docblock.as_deref());

    Ok(Some(SignatureHelp {
        signatures: vec![sig],
        active_signature: Some(0),
        active_parameter: Some(active_param),
    }))
}

/// Walk backwards from cursor to find the enclosing function call and count commas for active parameter.
fn find_call_context(source: &str, offset: usize) -> Option<(String, u32)> {
    let before = &source[..offset.min(source.len())];

    // First, figure out which characters are inside strings by scanning forward.
    let bytes = before.as_bytes();
    let mut in_string = vec![false; bytes.len()];
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\'' || bytes[i] == b'"' {
            let quote = bytes[i];
            in_string[i] = true;
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                in_string[i] = true;
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                    in_string[i] = true;
                }
                i += 1;
            }
            if i < bytes.len() {
                in_string[i] = true; // closing quote
            }
        }
        i += 1;
    }

    // Now walk backwards, skipping characters inside strings.
    let mut depth = 0i32;
    let mut commas = 0u32;
    let mut j = bytes.len();

    while j > 0 {
        j -= 1;
        if in_string[j] {
            continue;
        }
        match bytes[j] {
            b')' => depth += 1,
            b'(' => {
                if depth == 0 {
                    let before_paren = &before[..j];
                    let name = extract_function_name(before_paren)?;
                    return Some((name, commas));
                }
                depth -= 1;
            }
            b',' if depth == 0 => commas += 1,
            _ => {}
        }
    }

    None
}

/// Extract the function/method name immediately before the opening parenthesis.
fn extract_function_name(before_paren: &str) -> Option<String> {
    let trimmed = before_paren.trim_end();
    let start = trimmed
        .rfind(|c: char| !c.is_alphanumeric() && c != '_' && c != '\\')
        .map(|i| i + 1)
        .unwrap_or(0);
    let name = &trimmed[start..];
    if name.is_empty() { None } else { Some(name.to_string()) }
}

/// Resolve a function name to its metadata.
fn resolve_function_meta<'a, 'arena>(
    name: &str,
    resolved_names: &'a mago_names::ResolvedNames<'arena>,
    codebase: &'a mago_codex::metadata::CodebaseMetadata,
    program: &'a mago_syntax::ast::Program<'arena>,
) -> Option<&'a FunctionLikeMetadata> {
    // Try to find a matching identifier in the resolved names.
    for statement in program.statements.iter() {
        if let mago_syntax::ast::Statement::Namespace(ns) = statement {
            let stmts = match &ns.body {
                mago_syntax::ast::ast::NamespaceBody::Implicit(body) => &body.statements,
                mago_syntax::ast::ast::NamespaceBody::BraceDelimited(block) => &block.statements,
            };
            for stmt in stmts.iter() {
                if let Some(meta) = try_resolve_function_in_statement(stmt, name, resolved_names, codebase) {
                    return Some(meta);
                }
            }
        }
        if let Some(meta) = try_resolve_function_in_statement(statement, name, resolved_names, codebase) {
            return Some(meta);
        }
    }

    // Direct lookup by name as fallback.
    let key = (mago_atom::Atom::from(""), mago_atom::ascii_lowercase_atom(name));
    codebase.function_likes.get(&key)
}

fn try_resolve_function_in_statement<'a, 'arena>(
    statement: &'a mago_syntax::ast::Statement<'arena>,
    name: &str,
    resolved_names: &'a mago_names::ResolvedNames<'arena>,
    codebase: &'a mago_codex::metadata::CodebaseMetadata,
) -> Option<&'a FunctionLikeMetadata> {
    if let mago_syntax::ast::Statement::Function(func) = statement {
        if func.name.value == name {
            if let Some(fqn) = resolved_names.resolve(&func.name) {
                let key = (mago_atom::Atom::from(""), mago_atom::ascii_lowercase_atom(fqn));
                return codebase.function_likes.get(&key);
            }
        }
    }
    None
}

/// Find the docblock for a function by name.
fn find_function_docblock<'arena>(
    program: &'arena Program<'arena>,
    file: &File,
    func_name: &str,
) -> Option<String> {
    for statement in program.statements.iter() {
        if let mago_syntax::ast::Statement::Function(func) = statement {
            if func.name.value == func_name {
                if let Some(trivia) = get_docblock_for_node(program, file, func) {
                    return Some(trivia.value.to_string());
                }
            }
        }
        if let mago_syntax::ast::Statement::Namespace(ns) = statement {
            let stmts = match &ns.body {
                mago_syntax::ast::ast::NamespaceBody::Implicit(body) => &body.statements,
                mago_syntax::ast::ast::NamespaceBody::BraceDelimited(block) => &block.statements,
            };
            for stmt in stmts.iter() {
                if let mago_syntax::ast::Statement::Function(func) = stmt {
                    if func.name.value == func_name {
                        if let Some(trivia) = get_docblock_for_node(program, file, func) {
                            return Some(trivia.value.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

fn build_signature(meta: &FunctionLikeMetadata, raw_docblock: Option<&str>) -> SignatureInformation {
    let name = meta.original_name.as_ref().map(|a| a.as_str()).unwrap_or("unknown");

    let params: Vec<String> = meta
        .parameters
        .iter()
        .map(|p| {
            let mut s = String::new();
            if let Some(tm) = &p.type_metadata {
                s.push_str(&tm.type_union.get_id().to_string());
                s.push(' ');
            }
            s.push_str(&p.name.0.to_string());
            s
        })
        .collect();

    let label = format!("{}({})", name, params.join(", "));

    let parameters = Some(
        meta.parameters
            .iter()
            .map(|p| {
                let mut param_label = String::new();
                if let Some(tm) = &p.type_metadata {
                    param_label.push_str(&tm.type_union.get_id().to_string());
                    param_label.push(' ');
                }
                param_label.push_str(&p.name.0.to_string());

                let param_doc = raw_docblock
                    .and_then(|raw| super::hover::extract_param_doc(raw, &p.name.0.to_string()))
                    .map(|desc| lsp_types::Documentation::String(desc));

                ParameterInformation {
                    label: ParameterLabel::Simple(param_label),
                    documentation: param_doc,
                }
            })
            .collect(),
    );

    let documentation = raw_docblock
        .map(super::hover::clean_docblock)
        .filter(|d| !d.is_empty())
        .map(|d| {
            lsp_types::Documentation::MarkupContent(lsp_types::MarkupContent {
                kind: lsp_types::MarkupKind::Markdown,
                value: d,
            })
        });

    SignatureInformation {
        label,
        documentation,
        parameters,
        active_parameter: None,
    }
}

/// Resolve a class name to its FQN.
fn resolve_class_name<'arena>(
    name: &str,
    resolved_names: &mago_names::ResolvedNames<'arena>,
    program: &mago_syntax::ast::Program<'arena>,
) -> Option<String> {
    for stmt in program.statements.iter() {
        if let mago_syntax::ast::Statement::Class(class) = stmt {
            if class.name.value == name {
                return resolved_names.resolve(&class.name).map(|s| s.to_string());
            }
        }
        if let mago_syntax::ast::Statement::Namespace(ns) = stmt {
            let stmts = match &ns.body {
                mago_syntax::ast::ast::NamespaceBody::Implicit(body) => &body.statements,
                mago_syntax::ast::ast::NamespaceBody::BraceDelimited(block) => &block.statements,
            };
            for s in stmts.iter() {
                if let mago_syntax::ast::Statement::Class(class) = s {
                    if class.name.value == name {
                        return resolved_names.resolve(&class.name).map(|s| s.to_string());
                    }
                }
            }
        }
    }
    Some(name.to_string())
}

/// Build signature from AST for a class constructor.
fn build_constructor_signature_from_ast<'arena>(
    program: &'arena Program<'arena>,
    class_name: &str,
    resolved_names: &mago_names::ResolvedNames<'arena>,
    file: &File,
) -> Option<SignatureInformation> {
    for stmt in program.statements.iter() {
        if let mago_syntax::ast::Statement::Class(class) = stmt {
            let fqn = resolved_names.resolve(&class.name).unwrap_or(class.name.value);
            if fqn != class_name && class.name.value != class_name {
                continue;
            }
            for member in class.members.iter() {
                if let mago_syntax::ast::ast::ClassLikeMember::Method(method) = member {
                    if method.name.value == "__construct" {
                        return Some(build_signature_from_method(method, class_name, file, program));
                    }
                }
            }
        }
    }
    None
}

/// Build signature from AST for a method.
fn build_method_signature_from_ast<'arena>(
    program: &'arena Program<'arena>,
    class_fqn: &str,
    method_name: &str,
    resolved_names: &mago_names::ResolvedNames<'arena>,
    file: &File,
) -> Option<SignatureInformation> {
    for stmt in program.statements.iter() {
        if let mago_syntax::ast::Statement::Class(class) = stmt {
            let fqn = resolved_names.resolve(&class.name).unwrap_or(class.name.value);
            if fqn != class_fqn {
                continue;
            }
            for member in class.members.iter() {
                if let mago_syntax::ast::ast::ClassLikeMember::Method(method) = member {
                    if method.name.value == method_name {
                        return Some(build_signature_from_method(method, &format!("{}::{}", class_fqn, method_name), file, program));
                    }
                }
            }
        }
    }
    None
}

/// Build signature from AST for a global function.
fn build_function_signature_from_ast<'arena>(
    program: &'arena Program<'arena>,
    func_name: &str,
    file: &File,
) -> Option<SignatureInformation> {
    use mago_span::HasSpan;
    for stmt in program.statements.iter() {
        if let mago_syntax::ast::Statement::Function(func) = stmt {
            if func.name.value == func_name {
                let params = build_params_from_parameter_list(&func.parameter_list, file, program);
                let label = format!("{}({})", func_name, params.iter().map(|p| {
                    if let ParameterLabel::Simple(s) = &p.label { s.clone() } else { String::new() }
                }).collect::<Vec<_>>().join(", "));

                let doc = get_docblock_for_node(program, file, func)
                    .map(|t| t.value)
                    .map(super::hover::clean_docblock)
                    .filter(|d| !d.is_empty())
                    .map(|d| lsp_types::Documentation::MarkupContent(lsp_types::MarkupContent {
                        kind: lsp_types::MarkupKind::Markdown,
                        value: d,
                    }));

                return Some(SignatureInformation {
                    label,
                    documentation: doc,
                    parameters: Some(params),
                    active_parameter: None,
                });
            }
        }
    }
    None
}

fn build_signature_from_method<'arena>(
    method: &mago_syntax::ast::ast::class_like::method::Method<'arena>,
    display_name: &str,
    file: &File,
    program: &'arena Program<'arena>,
) -> SignatureInformation {
    use mago_span::HasSpan;
    let params = build_params_from_parameter_list(&method.parameter_list, file, program);
    let label = format!("{}({})", display_name, params.iter().map(|p| {
        if let ParameterLabel::Simple(s) = &p.label { s.clone() } else { String::new() }
    }).collect::<Vec<_>>().join(", "));

    let doc = get_docblock_for_node(program, file, method)
        .map(|t| t.value)
        .map(super::hover::clean_docblock)
        .filter(|d| !d.is_empty())
        .map(|d| lsp_types::Documentation::MarkupContent(lsp_types::MarkupContent {
            kind: lsp_types::MarkupKind::Markdown,
            value: d,
        }));

    SignatureInformation {
        label,
        documentation: doc,
        parameters: Some(params),
        active_parameter: None,
    }
}

fn build_params_from_parameter_list<'arena>(
    param_list: &mago_syntax::ast::ast::function_like::parameter::FunctionLikeParameterList<'arena>,
    file: &File,
    program: &'arena Program<'arena>,
) -> Vec<ParameterInformation> {
    use mago_span::HasSpan;
    param_list.parameters.iter().map(|p| {
        let start = p.span().start.offset as usize;
        let end = p.span().end_offset() as usize;
        let param_text = if end <= file.contents.len() {
            file.contents[start..end].to_string()
        } else {
            format!("${}", p.variable.name)
        };
        ParameterInformation {
            label: ParameterLabel::Simple(param_text),
            documentation: None,
        }
    }).collect()
}
