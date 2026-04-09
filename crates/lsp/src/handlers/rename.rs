use std::sync::Arc;

use bumpalo::Bump;
use lsp_types::RenameParams;
use lsp_types::TextEdit;
use lsp_types::Uri;
use lsp_types::WorkspaceEdit;

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

    // Dispatch based on the kind of symbol found.
    match &symbol {
        SymbolAt::ClassLike { fqn, .. } => {
            let short = fqn.rsplit('\\').next().unwrap_or(fqn);
            rename_fqn(state, &fqn.to_lowercase(), short, new_name)
        }
        SymbolAt::Function { fqn, .. } => {
            let short = fqn.rsplit('\\').next().unwrap_or(fqn);
            rename_fqn(state, &fqn.to_lowercase(), short, new_name)
        }
        SymbolAt::Method { method_name, .. } => {
            rename_member(state, method_name, new_name)
        }
        SymbolAt::Property { property_name, .. } => {
            rename_member(state, property_name, new_name)
        }
        SymbolAt::ClassConstant { constant_name, .. } => {
            rename_member(state, constant_name, new_name)
        }
        SymbolAt::Variable { name, .. } => {
            rename_variable(state, uri, &file, name, new_name)
        }
        SymbolAt::Unknown => Ok(None),
    }
}

/// Find the length of a PHP identifier starting at the given position.
fn find_identifier_length(s: &str) -> usize {
    s.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '\\')
        .map(|c| c.len_utf8())
        .sum()
}

/// Rename all references to a fully-qualified name (class-like or function) across all files.
fn rename_fqn(
    state: &LspState,
    target_fqn: &str,
    _old_short_name: &str,
    new_name: &str,
) -> Result<Option<WorkspaceEdit>, ServerError> {
    let db = state.database();
    let mut changes: std::collections::HashMap<lsp_types::Uri, Vec<TextEdit>> =
        std::collections::HashMap::new();

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

                let source_text = &source_file.contents[ref_offset as usize..];
                let actual_len = find_identifier_length(source_text);
                if actual_len == 0 {
                    continue;
                }

                let ident_text =
                    &source_file.contents[ref_offset as usize..ref_offset as usize + actual_len];
                if let Some(short_start) = ident_text.rfind('\\') {
                    let abs_start = ref_offset + short_start as u32 + 1;
                    let s = convert::offset_to_lsp_position(&source_file, abs_start);
                    let e = convert::offset_to_lsp_position(
                        &source_file,
                        ref_offset + actual_len as u32,
                    );
                    changes.entry(file_uri.clone()).or_default().push(TextEdit {
                        range: lsp_types::Range { start: s, end: e },
                        new_text: new_name.to_string(),
                    });
                } else {
                    let start = convert::offset_to_lsp_position(&source_file, ref_offset);
                    let end = convert::offset_to_lsp_position(
                        &source_file,
                        ref_offset + actual_len as u32,
                    );
                    changes.entry(file_uri.clone()).or_default().push(TextEdit {
                        range: lsp_types::Range { start, end },
                        new_text: new_name.to_string(),
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

/// Rename all references to a method, property, or class constant name across all files.
///
/// Scans file contents for occurrences of the member name preceded by `->` or `::`.
fn rename_member(
    state: &LspState,
    member_name: &str,
    new_name: &str,
) -> Result<Option<WorkspaceEdit>, ServerError> {
    let db = state.database();
    let mut changes: std::collections::HashMap<lsp_types::Uri, Vec<TextEdit>> =
        std::collections::HashMap::new();

    let search_name = member_name.strip_prefix('$').unwrap_or(member_name);

    for source_file in db.files() {
        if source_file.file_type == FileType::Builtin {
            continue;
        }

        let contents = &source_file.contents;
        let mut search_start = 0usize;
        while let Some(pos) = contents[search_start..].find(search_name) {
            let abs_pos = search_start + pos;

            let is_member_access = if abs_pos >= 2 {
                let prefix = &contents[abs_pos - 2..abs_pos];
                prefix == "->" || prefix == "::"
            } else {
                false
            };

            let end_pos = abs_pos + search_name.len();
            let is_word_boundary = end_pos >= contents.len()
                || !contents.as_bytes()[end_pos].is_ascii_alphanumeric()
                    && contents.as_bytes()[end_pos] != b'_';

            if is_member_access && is_word_boundary {
                let start = convert::offset_to_lsp_position(&source_file, abs_pos as u32);
                let end = convert::offset_to_lsp_position(&source_file, end_pos as u32);

                if let Some(file_uri) = state.uri_for_file_id(&source_file.id) {
                    // Strip the `$` from the new name if the original didn't have it.
                    let replacement = new_name.strip_prefix('$').unwrap_or(new_name);
                    changes.entry(file_uri.clone()).or_default().push(TextEdit {
                        range: lsp_types::Range { start, end },
                        new_text: replacement.to_string(),
                    });
                }
            }

            search_start = abs_pos + search_name.len().max(1);
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

/// Rename all references to a variable within the same file.
fn rename_variable(
    _state: &LspState,
    uri: &Uri,
    file: &Arc<File>,
    var_name: &str,
    new_name: &str,
) -> Result<Option<WorkspaceEdit>, ServerError> {
    let mut edits = Vec::new();
    let contents = &file.contents;

    let search_name = if var_name.starts_with('$') {
        var_name.to_string()
    } else {
        format!("${}", var_name)
    };

    let replacement = if new_name.starts_with('$') {
        new_name.to_string()
    } else {
        format!("${}", new_name)
    };

    let mut search_start = 0usize;
    while let Some(pos) = contents[search_start..].find(&search_name) {
        let abs_pos = search_start + pos;
        let end_pos = abs_pos + search_name.len();

        let is_word_boundary = end_pos >= contents.len()
            || !contents.as_bytes()[end_pos].is_ascii_alphanumeric()
                && contents.as_bytes()[end_pos] != b'_';

        if is_word_boundary {
            let start = convert::offset_to_lsp_position(file, abs_pos as u32);
            let end = convert::offset_to_lsp_position(file, end_pos as u32);

            edits.push(TextEdit {
                range: lsp_types::Range { start, end },
                new_text: replacement.clone(),
            });
        }

        search_start = abs_pos + search_name.len().max(1);
    }

    if edits.is_empty() {
        return Ok(None);
    }

    let mut changes: std::collections::HashMap<lsp_types::Uri, Vec<TextEdit>> =
        std::collections::HashMap::new();
    changes.insert(uri.clone(), edits);

    Ok(Some(WorkspaceEdit {
        changes: Some(changes),
        ..Default::default()
    }))
}
