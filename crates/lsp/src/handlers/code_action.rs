use bumpalo::Bump;
use lsp_types::CodeAction;
use lsp_types::CodeActionKind;
use lsp_types::CodeActionOrCommand;
use lsp_types::CodeActionParams;
use lsp_types::TextEdit;
use lsp_types::WorkspaceEdit;

use mago_database::DatabaseReader;
use mago_names::resolver::NameResolver;
use mago_reporting::AnnotationKind;
use mago_span::HasSpan;
use mago_syntax::ast::Statement;
use mago_syntax::ast::ast::ClassLikeMember;
use mago_syntax::comments::docblock::get_docblock_for_node;
use mago_syntax::parser::parse_file_content;
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

    // Add refactoring code actions (Generate PHPDoc, Extract Variable).
    let refactoring_actions = generate_refactoring_actions(state, &params);
    actions.extend(refactoring_actions);

    Ok(if actions.is_empty() { None } else { Some(actions) })
}

/// Generate refactoring code actions based on cursor position.
fn generate_refactoring_actions(
    state: &LspState,
    params: &CodeActionParams,
) -> Vec<CodeActionOrCommand> {
    let mut actions = Vec::new();
    let uri = &params.text_document.uri;
    let range = params.range;

    let Some(file_id) = state.file_id_for_uri(uri) else { return actions };
    let Some(file) = state.get_file(&file_id) else { return actions };

    let arena = Bump::new();
    let program = parse_file_content(&arena, file.id, &file.contents);

    // Generate PHPDoc action
    if let Some(action) = generate_phpdoc_action(uri, &file, program, range) {
        actions.push(CodeActionOrCommand::CodeAction(action));
    }

    // Extract Variable action (when text is selected)
    if range.start != range.end {
        if let Some(action) = extract_variable_action(uri, &file, range) {
            actions.push(CodeActionOrCommand::CodeAction(action));
        }
    }

    actions
}

/// Generate a PHPDoc stub for a function/method/class at the cursor.
fn generate_phpdoc_action(
    uri: &lsp_types::Uri,
    file: &std::sync::Arc<mago_database::file::File>,
    program: &mago_syntax::ast::Program<'_>,
    range: lsp_types::Range,
) -> Option<CodeAction> {
    let offset = convert::lsp_position_to_offset(file, range.start);

    for stmt in program.statements.iter() {
        if !stmt.span().has_offset(offset) {
            continue;
        }

        match stmt {
            Statement::Function(func) => {
                // Check if already has a docblock
                if get_docblock_for_node(program, file, func).is_some() {
                    continue;
                }
                let doc = build_function_docblock(file, &func.parameter_list, func.return_type_hint.as_ref());
                let insert_pos = convert::offset_to_lsp_position(file, func.span().start.offset);
                return Some(build_docblock_action(uri, &doc, insert_pos));
            }
            Statement::Class(class) => {
                // Check if cursor is on a method
                for member in class.members.iter() {
                    if !member.span().has_offset(offset) {
                        continue;
                    }
                    if let ClassLikeMember::Method(method) = member {
                        if get_docblock_for_node(program, file, method).is_some() {
                            continue;
                        }
                        let doc = build_function_docblock(file, &method.parameter_list, method.return_type_hint.as_ref());
                        let insert_pos = convert::offset_to_lsp_position(file, member.span().start.offset);
                        return Some(build_docblock_action(uri, &doc, insert_pos));
                    }
                    if let ClassLikeMember::Property(_prop) = member {
                        if get_docblock_for_node(program, file, member).is_some() {
                            continue;
                        }
                        let doc = "/**\n * TODO: description\n */\n".to_string();
                        let insert_pos = convert::offset_to_lsp_position(file, member.span().start.offset);
                        return Some(build_docblock_action(uri, &doc, insert_pos));
                    }
                }
                // Check if class itself needs a docblock
                if get_docblock_for_node(program, file, class).is_none() {
                    let doc = "/**\n * TODO: description\n */\n".to_string();
                    let insert_pos = convert::offset_to_lsp_position(file, stmt.span().start.offset);
                    return Some(build_docblock_action(uri, &doc, insert_pos));
                }
            }
            _ => {}
        }
    }
    None
}

fn build_function_docblock(
    file: &std::sync::Arc<mago_database::file::File>,
    param_list: &mago_syntax::ast::ast::function_like::parameter::FunctionLikeParameterList<'_>,
    return_type: Option<&mago_syntax::ast::ast::function_like::r#return::FunctionLikeReturnTypeHint<'_>>,
) -> String {
    let mut lines = Vec::new();
    lines.push("/**".to_string());
    lines.push(" * TODO: description".to_string());
    lines.push(" *".to_string());

    for param in param_list.parameters.iter() {
        let name = format!("${}", param.variable.name);
        let type_str = if let Some(hint) = &param.hint {
            let start = hint.span().start.offset as usize;
            let end = hint.span().end_offset() as usize;
            if end <= file.contents.len() {
                file.contents[start..end].to_string()
            } else {
                "mixed".to_string()
            }
        } else {
            "mixed".to_string()
        };
        lines.push(format!(" * @param {} {} TODO", type_str, name));
    }

    if let Some(rth) = return_type {
        let start = rth.hint.span().start.offset as usize;
        let end = rth.hint.span().end_offset() as usize;
        let type_str = if end <= file.contents.len() {
            file.contents[start..end].to_string()
        } else {
            "mixed".to_string()
        };
        lines.push(format!(" * @return {} TODO", type_str));
    }

    lines.push(" */".to_string());
    lines.push(String::new()); // trailing newline
    lines.join("\n")
}

fn build_docblock_action(uri: &lsp_types::Uri, doc: &str, insert_pos: lsp_types::Position) -> CodeAction {
    // Find the indentation at the insertion line
    let insert_range = lsp_types::Range { start: insert_pos, end: insert_pos };
    let mut changes = std::collections::HashMap::new();
    changes.insert(uri.clone(), vec![TextEdit {
        range: insert_range,
        new_text: doc.to_string(),
    }]);

    CodeAction {
        title: "Generate PHPDoc".to_string(),
        kind: Some(CodeActionKind::SOURCE),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Extract the selected expression into a new variable.
fn extract_variable_action(
    uri: &lsp_types::Uri,
    file: &std::sync::Arc<mago_database::file::File>,
    range: lsp_types::Range,
) -> Option<CodeAction> {
    let start_offset = convert::lsp_position_to_offset(file, range.start) as usize;
    let end_offset = convert::lsp_position_to_offset(file, range.end) as usize;

    if end_offset <= start_offset || end_offset > file.contents.len() {
        return None;
    }

    let selected_text = &file.contents[start_offset..end_offset];
    // Don't extract if it's just whitespace or a simple variable
    let trimmed = selected_text.trim();
    if trimmed.is_empty() || trimmed.starts_with('$') && !trimmed.contains("->") {
        return None;
    }

    // Find the beginning of the current line for the variable declaration
    let line_start = file.contents[..start_offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let indentation = &file.contents[line_start..start_offset]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect::<String>();

    let var_declaration = format!("$variable = {};\n{}", trimmed, indentation);
    let line_start_pos = convert::offset_to_lsp_position(file, line_start as u32);

    let mut changes = std::collections::HashMap::new();
    changes.insert(uri.clone(), vec![
        // Insert variable declaration at the beginning of the line
        TextEdit {
            range: lsp_types::Range { start: line_start_pos, end: line_start_pos },
            new_text: format!("{}{}", indentation, var_declaration),
        },
        // Replace the selected expression with the variable reference
        TextEdit {
            range,
            new_text: "$variable".to_string(),
        },
    ]);

    Some(CodeAction {
        title: "Extract Variable".to_string(),
        kind: Some(CodeActionKind::REFACTOR_EXTRACT),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        ..Default::default()
    })
}

fn ranges_overlap(a: &lsp_types::Range, b: &lsp_types::Range) -> bool {
    a.start.line <= b.end.line && b.start.line <= a.end.line
}
