use lsp_server::Connection;
use lsp_types::notification::Notification;

use mago_database::DatabaseReader;
use mago_database::file::FileId;
use mago_reporting::IgnoreEntry;

use crate::convert;
use crate::state::LspState;

/// Publish diagnostics for the given file IDs to the client.
pub fn publish_diagnostics(state: &LspState, connection: &Connection, file_ids: &[FileId]) {
    let db = state.database();
    let ignored = &state.ignored_diagnostics;

    for file_id in file_ids {
        let Ok(file) = db.get(file_id) else {
            continue;
        };
        let Some(uri) = state.uri_for_file_id(file_id) else {
            continue;
        };

        let file_path = file.path.as_deref().map(|p| p.to_string_lossy().to_string());

        let mut diagnostics: Vec<lsp_types::Diagnostic> = match state.get_file_diagnostics(file_id) {
            Some(issues) => issues.iter()
                .filter(|issue| !is_ignored(issue, ignored, file_path.as_deref()))
                .filter_map(|issue| convert::issue_to_diagnostic(&file, issue))
                .collect(),
            None => vec![],
        };

        // Add embedded language diagnostics (SQL syntax errors, etc.).
        diagnostics.extend(super::embedded::get_embedded_diagnostics(&file));

        let params = lsp_types::PublishDiagnosticsParams {
            uri: uri.clone(),
            diagnostics,
            version: None,
        };

        let notification = lsp_server::Notification::new(
            lsp_types::notification::PublishDiagnostics::METHOD.to_string(),
            params,
        );

        if let Err(e) = connection.sender.send(lsp_server::Message::Notification(notification)) {
            tracing::error!("failed to publish diagnostics for {}: {e}", uri.as_str());
        }
    }
}

/// Check if an issue should be ignored based on the ignore rules from mago.toml.
fn is_ignored(issue: &mago_reporting::Issue, ignore: &[IgnoreEntry], file_path: Option<&str>) -> bool {
    let Some(code) = &issue.code else {
        return false;
    };

    for entry in ignore {
        match entry {
            IgnoreEntry::Code(ignored_code) => {
                if code == ignored_code {
                    return true;
                }
            }
            IgnoreEntry::Scoped { code: ignored_code, paths } => {
                if code == ignored_code {
                    if let Some(fp) = file_path {
                        let normalized = fp.replace('\\', "/");
                        if paths.iter().any(|p| normalized.contains(p)) {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}
