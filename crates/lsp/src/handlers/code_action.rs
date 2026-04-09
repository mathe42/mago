use lsp_types::CodeAction;
use lsp_types::CodeActionKind;
use lsp_types::CodeActionOrCommand;
use lsp_types::CodeActionParams;
use lsp_types::TextEdit;
use lsp_types::WorkspaceEdit;

use mago_database::DatabaseReader;
use mago_reporting::AnnotationKind;
use mago_text_edit::Safety;

use crate::convert;
use crate::error::ServerError;
use crate::state::LspState;

/// Handle `textDocument/codeAction`.
///
/// Produces quick-fix code actions from issue edits (auto-fix suggestions
/// from the linter and analyzer).
pub fn handle_code_action(
    state: &LspState,
    params: CodeActionParams,
) -> Result<Option<Vec<CodeActionOrCommand>>, ServerError> {
    let uri = &params.text_document.uri;
    let range = params.range;

    let Some(file_id) = state.file_id_for_uri(uri) else {
        return Ok(None);
    };
    let Some(file) = state.get_file(&file_id) else {
        return Ok(None);
    };

    let Some(issues) = state.get_file_diagnostics(&file_id) else {
        return Ok(Some(vec![]));
    };

    let db = state.database();
    let mut actions = Vec::new();

    for issue in issues.iter() {
        // Check if this issue overlaps with the requested range.
        let primary_span = issue
            .annotations
            .iter()
            .find(|a| a.kind == AnnotationKind::Primary)
            .map(|a| a.span);

        let Some(span) = primary_span else {
            continue;
        };

        let issue_range = convert::span_to_range(&file, span);
        if !ranges_overlap(&issue_range, &range) {
            continue;
        }

        // Check if the issue has fixable edits.
        if issue.edits.is_empty() {
            continue;
        }

        for (&edit_file_id, edits) in &issue.edits {
            if edits.is_empty() {
                continue;
            }

            // Only include safe and potentially-unsafe edits.
            let safe_edits: Vec<&mago_text_edit::TextEdit> = edits
                .iter()
                .filter(|e| matches!(e.safety, Safety::Safe | Safety::PotentiallyUnsafe))
                .collect();

            if safe_edits.is_empty() {
                continue;
            }

            let edit_file = match db.get(&edit_file_id) {
                Ok(f) => f,
                Err(_) => continue,
            };
            let edit_uri = match state.uri_for_file_id(&edit_file_id) {
                Some(u) => u.clone(),
                None => continue,
            };

            let lsp_edits: Vec<TextEdit> = safe_edits
                .iter()
                .map(|e| {
                    let start = convert::offset_to_lsp_position(&edit_file, e.range.start);
                    let end = convert::offset_to_lsp_position(&edit_file, e.range.end);
                    TextEdit {
                        range: lsp_types::Range { start, end },
                        new_text: e.new_text.clone(),
                    }
                })
                .collect();

            let mut changes = std::collections::HashMap::new();
            changes.insert(edit_uri, lsp_edits);

            let title = if let Some(ref code) = issue.code {
                format!("Fix: {} ({})", issue.message, code)
            } else {
                format!("Fix: {}", issue.message)
            };

            // Truncate title to reasonable length.
            let title = if title.len() > 80 {
                format!("{}...", &title[..77])
            } else {
                title
            };

            let is_preferred = safe_edits.iter().all(|e| e.safety == Safety::Safe);

            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title,
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: None,
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                is_preferred: Some(is_preferred),
                ..Default::default()
            }));
        }
    }

    Ok(if actions.is_empty() { None } else { Some(actions) })
}

fn ranges_overlap(a: &lsp_types::Range, b: &lsp_types::Range) -> bool {
    a.start.line <= b.end.line && b.start.line <= a.end.line
}
