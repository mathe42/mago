use bumpalo::Bump;
use lsp_types::GotoDefinitionParams;
use lsp_types::GotoDefinitionResponse;
use lsp_types::Location;

use mago_database::DatabaseReader;
use mago_database::file::File;
use mago_names::resolver::NameResolver;
use mago_span::HasSpan;
use mago_syntax::parser::parse_file_content;

use crate::convert;
use crate::error::ServerError;
use crate::navigate;
use crate::navigate::SymbolAt;
use crate::state::LspState;

/// Handle `textDocument/definition`.
pub fn handle_goto_definition(
    state: &LspState,
    params: GotoDefinitionParams,
) -> Result<Option<GotoDefinitionResponse>, ServerError> {
    let uri = &params.text_document_position_params.text_document.uri;
    let position = params.text_document_position_params.position;

    let Some(file_id) = state.file_id_for_uri(uri) else {
        return Ok(None);
    };
    let Some(file) = state.get_file(&file_id) else {
        return Ok(None);
    };

    let offset = convert::lsp_position_to_offset(&file, position);

    // Parse the current file and resolve names.
    let arena = Bump::new();
    let program = parse_file_content(&arena, file.id, &file.contents);
    let resolved_names = NameResolver::new(&arena).resolve(program);

    let codebase = state.codebase();
    let symbol = navigate::find_symbol_at_offset(program, &resolved_names, codebase, offset);

    let location = match symbol {
        SymbolAt::ClassLike { fqn, .. } => {
            if let Some(meta) = codebase.get_class_like(fqn) {
                resolve_span_to_location(state, meta.name_span.unwrap_or(meta.span))
            } else {
                None
            }
        }
        SymbolAt::Function { fqn, .. } => {
            if let Some(meta) = codebase.get_function(fqn) {
                resolve_span_to_location(state, meta.name_span.unwrap_or(meta.span))
            } else {
                None
            }
        }
        SymbolAt::Method { ref class_fqn, ref method_name, .. } if !class_fqn.is_empty() => {
            if let Some(meta) = codebase.get_declaring_method(class_fqn, method_name) {
                resolve_span_to_location(state, meta.name_span.unwrap_or(meta.span))
            } else {
                // Fallback: search AST
                find_method_in_ast(&arena, &file, class_fqn, method_name)
                    .map(|range| Location { uri: uri.clone(), range })
            }
        }
        SymbolAt::Property { ref class_fqn, ref property_name, .. } if !class_fqn.is_empty() => {
            let lookup_name = if property_name.starts_with('$') {
                property_name.clone()
            } else {
                format!("${}", property_name)
            };
            if let Some(meta) = codebase.get_declaring_property(class_fqn, &lookup_name) {
                // Try span, then name_span as fallback
                meta.span.and_then(|span| resolve_span_to_location(state, span))
                    .or_else(|| meta.name_span.and_then(|span| resolve_span_to_location(state, span)))
            } else {
                tracing::debug!("property not found: {}::{}", class_fqn, lookup_name);
                // Fallback: search the current file's AST for the property definition
                find_property_in_ast(&arena, &file, class_fqn, &lookup_name)
                    .map(|range| Location { uri: uri.clone(), range })
            }
        }
        SymbolAt::ClassConstant { ref class_fqn, ref constant_name, .. } => {
            if let Some(meta) = codebase.get_class_constant(class_fqn, constant_name) {
                resolve_span_to_location(state, meta.span)
            } else {
                None
            }
        }
        SymbolAt::Variable { definition_span: Some(def_span), .. } => {
            let range = convert::span_to_range(&file, def_span);
            Some(Location { uri: uri.clone(), range })
        }
        _ => None,
    };

    Ok(location.map(GotoDefinitionResponse::Scalar))
}

/// Resolve a Span to an LSP Location by looking up the file in the database.
fn resolve_span_to_location(state: &LspState, span: mago_span::Span) -> Option<Location> {
    let db = state.database();
    let file = db.get(&span.file_id).ok()?;
    let uri = state.uri_for_file_id(&span.file_id)?.clone();
    let range = convert::span_to_range(&file, span);
    Some(Location { uri, range })
}

/// Search the current file's AST for a property definition as fallback.
fn find_property_in_ast(
    arena: &Bump,
    file: &File,
    class_fqn: &str,
    property_name: &str, // with $ prefix
) -> Option<lsp_types::Range> {
    let program = parse_file_content(arena, file.id, &file.contents);
    let resolved_names = NameResolver::new(arena).resolve(program);

    for stmt in program.statements.iter() {
        if let mago_syntax::ast::Statement::Class(class) = stmt {
            let fqn = resolved_names.resolve(&class.name).unwrap_or(class.name.value);
            if fqn != class_fqn {
                continue;
            }
            for member in class.members.iter() {
                if let mago_syntax::ast::ast::ClassLikeMember::Property(prop) = member {
                    let vars = match prop {
                        mago_syntax::ast::ast::class_like::property::Property::Plain(p) => {
                            p.items.iter().map(|i| i.variable()).collect::<Vec<_>>()
                        }
                        mago_syntax::ast::ast::class_like::property::Property::Hooked(h) => {
                            vec![h.item.variable()]
                        }
                    };
                    for var in vars {
                        let var_name = if var.name.starts_with('$') {
                            var.name.to_string()
                        } else {
                            format!("${}", var.name)
                        };
                        if var_name == property_name {
                            return Some(convert::span_to_range(file, var.span()));
                        }
                    }
                }
            }
        }
    }
    None
}

/// Search the current file's AST for a method definition as fallback.
fn find_method_in_ast(
    arena: &Bump,
    file: &File,
    class_fqn: &str,
    method_name: &str,
) -> Option<lsp_types::Range> {
    let program = parse_file_content(arena, file.id, &file.contents);
    let resolved_names = NameResolver::new(arena).resolve(program);

    for stmt in program.statements.iter() {
        if let mago_syntax::ast::Statement::Class(class) = stmt {
            let fqn = resolved_names.resolve(&class.name).unwrap_or(class.name.value);
            if fqn != class_fqn {
                continue;
            }
            for member in class.members.iter() {
                if let mago_syntax::ast::ast::ClassLikeMember::Method(method) = member {
                    if method.name.value == method_name {
                        return Some(convert::span_to_range(file, method.name.span()));
                    }
                }
            }
        }
    }
    None
}
