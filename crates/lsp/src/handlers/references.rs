use std::sync::Arc;

use bumpalo::Bump;
use lsp_types::Location;
use lsp_types::ReferenceParams;
use lsp_types::Uri;

use mago_database::DatabaseReader;
use mago_database::file::File;
use mago_database::file::FileType;
use mago_names::resolver::NameResolver;

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

    // Dispatch based on the kind of symbol found.
    match &symbol {
        SymbolAt::ClassLike { fqn, .. } => {
            find_fqn_references(state, &fqn.to_lowercase())
        }
        SymbolAt::Function { fqn, .. } => {
            find_fqn_references(state, &fqn.to_lowercase())
        }
        SymbolAt::Method { method_name, .. } => {
            find_member_references(state, method_name)
        }
        SymbolAt::Property { property_name, .. } => {
            find_member_references(state, property_name)
        }
        SymbolAt::ClassConstant { constant_name, .. } => {
            find_member_references(state, constant_name)
        }
        SymbolAt::Variable { name, .. } => {
            find_variable_references(state, uri, &file, &name)
        }
        SymbolAt::Unknown => Ok(None),
    }
}

/// Find references to a fully-qualified name (class-like or function) across all files.
fn find_fqn_references(
    state: &LspState,
    target_fqn: &str,
) -> Result<Option<Vec<Location>>, ServerError> {
    let db = state.database();
    let mut locations = Vec::new();

    for source_file in db.files() {
        if source_file.file_type == FileType::Builtin {
            continue;
        }

        let file_arena = Bump::new();
        let file_program = parse_file_content(&file_arena, source_file.id, &source_file.contents);
        let file_resolved = NameResolver::new(&file_arena).resolve(file_program);

        for (&offset, &(name, _)) in file_resolved.all() {
            if name.to_lowercase() == target_fqn {
                let pos = convert::offset_to_lsp_position(&source_file, offset);
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

/// Find references to a method, property, or class constant name across all files.
///
/// Scans file contents for occurrences of the member name preceded by `->` or `::`.
fn find_member_references(
    state: &LspState,
    member_name: &str,
) -> Result<Option<Vec<Location>>, ServerError> {
    let db = state.database();
    let mut locations = Vec::new();

    // Strip leading `$` from property names for the text search.
    let search_name = member_name.strip_prefix('$').unwrap_or(member_name);

    for source_file in db.files() {
        if source_file.file_type == FileType::Builtin {
            continue;
        }

        let contents = &source_file.contents;
        let mut search_start = 0usize;
        while let Some(pos) = contents[search_start..].find(search_name) {
            let abs_pos = search_start + pos;

            // Check that the match is preceded by `->` or `::`.
            let is_member_access = if abs_pos >= 2 {
                let prefix = &contents[abs_pos - 2..abs_pos];
                prefix == "->" || prefix == "::"
            } else {
                false
            };

            // Also check that the character after the name is not alphanumeric or `_`
            // (to avoid partial matches).
            let end_pos = abs_pos + search_name.len();
            let is_word_boundary = end_pos >= contents.len()
                || !contents.as_bytes()[end_pos].is_ascii_alphanumeric()
                    && contents.as_bytes()[end_pos] != b'_';

            if is_member_access && is_word_boundary {
                let start = convert::offset_to_lsp_position(&source_file, abs_pos as u32);
                let end = convert::offset_to_lsp_position(&source_file, end_pos as u32);

                if let Some(file_uri) = state.uri_for_file_id(&source_file.id) {
                    locations.push(Location {
                        uri: file_uri.clone(),
                        range: lsp_types::Range { start, end },
                    });
                }
            }

            search_start = abs_pos + search_name.len().max(1);
        }
    }

    if locations.is_empty() { Ok(None) } else { Ok(Some(locations)) }
}

/// Find references to a variable name within the same file.
fn find_variable_references(
    _state: &LspState,
    uri: &Uri,
    file: &Arc<File>,
    var_name: &str,
) -> Result<Option<Vec<Location>>, ServerError> {
    let mut locations = Vec::new();
    let contents = &file.contents;

    // Ensure the search name has a `$` prefix.
    let search_name = if var_name.starts_with('$') {
        var_name.to_string()
    } else {
        format!("${}", var_name)
    };

    let mut search_start = 0usize;
    while let Some(pos) = contents[search_start..].find(&search_name) {
        let abs_pos = search_start + pos;
        let end_pos = abs_pos + search_name.len();

        // Ensure the match is at a word boundary (next char is not alphanumeric or `_`).
        let is_word_boundary = end_pos >= contents.len()
            || !contents.as_bytes()[end_pos].is_ascii_alphanumeric()
                && contents.as_bytes()[end_pos] != b'_';

        if is_word_boundary {
            let start = convert::offset_to_lsp_position(file, abs_pos as u32);
            let end = convert::offset_to_lsp_position(file, end_pos as u32);

            locations.push(Location {
                uri: uri.clone(),
                range: lsp_types::Range { start, end },
            });
        }

        search_start = abs_pos + search_name.len().max(1);
    }

    if locations.is_empty() { Ok(None) } else { Ok(Some(locations)) }
}
