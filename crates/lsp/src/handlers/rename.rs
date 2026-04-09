use bumpalo::Bump;
use lsp_types::RenameParams;
use lsp_types::TextEdit;
use lsp_types::WorkspaceEdit;

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

/// Handle `textDocument/rename`.
///
/// Renames all references to the symbol at the cursor position.
pub fn handle_rename(
    state: &LspState,
    params: RenameParams,
) -> Result<Option<WorkspaceEdit>, ServerError> {
    let uri = &params.text_document_position.text_document.uri;
    let position = params.text_document_position.position;
    let new_name = &params.new_name;

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

    // Get the short name (last segment) and the FQN to search for.
    let (target_fqn, old_short_name) = match &symbol {
        SymbolAt::ClassLike { fqn, .. } => {
            let short = fqn.rsplit('\\').next().unwrap_or(fqn);
            (fqn.to_lowercase(), short.to_string())
        }
        SymbolAt::Function { fqn, .. } => {
            let short = fqn.rsplit('\\').next().unwrap_or(fqn);
            (fqn.to_lowercase(), short.to_string())
        }
        _ => return Ok(None),
    };

    let db = state.database();
    let mut changes: std::collections::HashMap<lsp_types::Uri, Vec<TextEdit>> = std::collections::HashMap::new();

    for source_file in db.files() {
        if source_file.file_type == FileType::Builtin {
            continue;
        }

        let file_arena = Bump::new();
        let file_program = parse_file_content(&file_arena, source_file.id, &source_file.contents);
        let file_resolved = NameResolver::new(&file_arena).resolve(file_program);

        for (&ref_offset, &(name, _)) in file_resolved.all() {
            if name.to_lowercase() == target_fqn {
                let Some(file_uri) = state.uri_for_file_id(&source_file.id) else {
                    continue;
                };

                // Find the short name in the source at this position.
                let source_text = &source_file.contents[ref_offset as usize..];
                let actual_len = find_identifier_length(source_text);
                if actual_len == 0 {
                    continue;
                }

                let start = convert::offset_to_lsp_position(&source_file, ref_offset);
                let end = convert::offset_to_lsp_position(&source_file, ref_offset + actual_len as u32);

                // Only rename the last segment (short name), not the whole FQN.
                // Find where the short name starts within the identifier at this position.
                let ident_text = &source_file.contents[ref_offset as usize..ref_offset as usize + actual_len];
                if let Some(short_start) = ident_text.rfind('\\') {
                    let abs_start = ref_offset + short_start as u32 + 1; // skip backslash
                    let s = convert::offset_to_lsp_position(&source_file, abs_start);
                    let e = convert::offset_to_lsp_position(&source_file, ref_offset + actual_len as u32);
                    changes.entry(file_uri.clone()).or_default().push(TextEdit {
                        range: lsp_types::Range { start: s, end: e },
                        new_text: new_name.clone(),
                    });
                } else {
                    changes.entry(file_uri.clone()).or_default().push(TextEdit {
                        range: lsp_types::Range { start, end },
                        new_text: new_name.clone(),
                    });
                }
            }
        }
    }

    if changes.is_empty() {
        return Ok(None);
    }

    Ok(Some(WorkspaceEdit {
        changes: Some(changes),
        ..Default::default()
    }))
}

/// Find the length of a PHP identifier starting at the given position.
fn find_identifier_length(s: &str) -> usize {
    s.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '\\')
        .map(|c| c.len_utf8())
        .sum()
}
