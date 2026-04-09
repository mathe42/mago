use lsp_types::Location;
use lsp_types::SymbolKind;
use lsp_types::WorkspaceSymbolParams;

use mago_atom::empty_atom;
use mago_database::DatabaseReader;

use crate::convert;
use crate::error::ServerError;
use crate::state::LspState;

/// Handle `workspace/symbol`.
///
/// Returns symbols from the codebase that match the given query string
/// (case-insensitive substring match).
pub fn handle_workspace_symbol(
    state: &LspState,
    params: WorkspaceSymbolParams,
) -> Result<Option<Vec<lsp_types::SymbolInformation>>, ServerError> {
    let query = params.query.to_lowercase();
    let codebase = state.codebase();
    let db = state.database();

    let mut symbols = Vec::new();

    // Search class-like symbols (classes, interfaces, traits, enums).
    for (_key, meta) in &codebase.class_likes {
        let original_name = meta.original_name.as_str();
        if !query.is_empty() && !original_name.to_lowercase().contains(&query) {
            continue;
        }

        let kind = match meta.kind {
            mago_codex::symbol::SymbolKind::Class => SymbolKind::CLASS,
            mago_codex::symbol::SymbolKind::Interface => SymbolKind::INTERFACE,
            mago_codex::symbol::SymbolKind::Trait => SymbolKind::CLASS,
            mago_codex::symbol::SymbolKind::Enum => SymbolKind::ENUM,
        };

        let span = meta.name_span.unwrap_or(meta.span);
        let Some(file) = db.get(&span.file_id).ok() else {
            continue;
        };
        let Some(uri) = state.uri_for_file_id(&span.file_id) else {
            continue;
        };

        let range = convert::span_to_range(&file, span);

        #[allow(deprecated)]
        symbols.push(lsp_types::SymbolInformation {
            name: original_name.to_string(),
            kind,
            tags: None,
            deprecated: None,
            location: Location {
                uri: uri.clone(),
                range,
            },
            container_name: None,
        });
    }

    // Search function-like symbols (global functions only).
    let empty = empty_atom();
    for ((scope, _name_key), meta) in &codebase.function_likes {
        // Only include global functions (scope is empty).
        if *scope != empty {
            continue;
        }

        let Some(original_name) = &meta.original_name else {
            continue;
        };
        let name_str = original_name.as_str();

        if !query.is_empty() && !name_str.to_lowercase().contains(&query) {
            continue;
        }

        let span = meta.name_span.unwrap_or(meta.span);
        let Some(file) = db.get(&span.file_id).ok() else {
            continue;
        };
        let Some(uri) = state.uri_for_file_id(&span.file_id) else {
            continue;
        };

        let range = convert::span_to_range(&file, span);

        #[allow(deprecated)]
        symbols.push(lsp_types::SymbolInformation {
            name: name_str.to_string(),
            kind: SymbolKind::FUNCTION,
            tags: None,
            deprecated: None,
            location: Location {
                uri: uri.clone(),
                range,
            },
            container_name: None,
        });
    }

    if symbols.is_empty() {
        Ok(None)
    } else {
        Ok(Some(symbols))
    }
}
