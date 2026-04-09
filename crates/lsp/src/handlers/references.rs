use bumpalo::Bump;
use lsp_types::Location;
use lsp_types::ReferenceParams;

use mago_database::DatabaseReader;
use mago_database::file::FileType;
use mago_names::resolver::NameResolver;
use mago_span::HasPosition;
use mago_syntax::parser::parse_file_content;

use crate::convert;
use crate::error::ServerError;
use crate::navigate;
use crate::navigate::SymbolAt;
use crate::state::LspState;

/// Handle `textDocument/references`.
///
/// Finds all references to the symbol at the cursor position across the workspace.
/// This requires scanning all files, so it may be slow on large workspaces.
pub fn handle_references(
    state: &LspState,
    params: ReferenceParams,
) -> Result<Option<Vec<Location>>, ServerError> {
    let uri = &params.text_document_position.text_document.uri;
    let position = params.text_document_position.position;

    let Some(file_id) = state.file_id_for_uri(uri) else {
        return Ok(None);
    };
    let Some(file) = state.get_file(&file_id) else {
        return Ok(None);
    };

    let offset = convert::lsp_position_to_offset(&file, position);

    // Parse and resolve names to find the target symbol.
    let arena = Bump::new();
    let program = parse_file_content(&arena, file.id, &file.contents);
    let resolved_names = NameResolver::new(&arena).resolve(program);

    let codebase = state.codebase();
    let symbol = navigate::find_symbol_at_offset(program, &resolved_names, codebase, offset);

    let target_fqn = match &symbol {
        SymbolAt::ClassLike { fqn, .. } => Some(fqn.to_lowercase()),
        SymbolAt::Function { fqn, .. } => Some(fqn.to_lowercase()),
        _ => None,
    };

    let Some(target_fqn) = target_fqn else {
        return Ok(None);
    };

    // Scan all host files for references to the target FQN.
    let db = state.database();
    let mut locations = Vec::new();

    for source_file in db.files() {
        if source_file.file_type == FileType::Builtin {
            continue;
        }

        let file_arena = Bump::new();
        let file_program = parse_file_content(&file_arena, source_file.id, &source_file.contents);
        let file_resolved = NameResolver::new(&file_arena).resolve(file_program);

        // Check all resolved names for a match.
        for (&offset, &(name, _)) in file_resolved.all() {
            if name.to_lowercase() == target_fqn {
                let pos = convert::offset_to_lsp_position(&source_file, offset);
                // Approximate end position (identifier length unknown, use a reasonable default).
                let end_offset = offset + name.split('\\').last().map(|s| s.len() as u32).unwrap_or(1);
                let end_pos = convert::offset_to_lsp_position(&source_file, end_offset);

                if let Some(file_uri) = state.uri_for_file_id(&source_file.id) {
                    locations.push(Location {
                        uri: file_uri.clone(),
                        range: lsp_types::Range { start: pos, end: end_pos },
                    });
                }
            }
        }
    }

    if locations.is_empty() { Ok(None) } else { Ok(Some(locations)) }
}
