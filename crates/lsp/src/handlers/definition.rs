use bumpalo::Bump;
use lsp_types::GotoDefinitionParams;
use lsp_types::GotoDefinitionResponse;
use lsp_types::Location;

use mago_database::DatabaseReader;
use mago_names::resolver::NameResolver;
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
                None
            }
        }
        SymbolAt::Property { ref class_fqn, ref property_name, .. } if !class_fqn.is_empty() => {
            if let Some(meta) = codebase.get_declaring_property(class_fqn, property_name) {
                meta.span.and_then(|span| resolve_span_to_location(state, span))
            } else {
                None
            }
        }
        SymbolAt::ClassConstant { ref class_fqn, ref constant_name, .. } => {
            if let Some(meta) = codebase.get_class_constant(class_fqn, constant_name) {
                resolve_span_to_location(state, meta.span)
            } else {
                None
            }
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
