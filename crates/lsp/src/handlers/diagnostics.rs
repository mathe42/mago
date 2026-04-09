use lsp_server::Connection;
use lsp_types::notification::Notification;

use mago_database::DatabaseReader;
use mago_database::file::FileId;

use crate::convert;
use crate::state::LspState;

/// Publish diagnostics for the given file IDs to the client.
pub fn publish_diagnostics(state: &LspState, connection: &Connection, file_ids: &[FileId]) {
    let db = state.database();

    for file_id in file_ids {
        let Ok(file) = db.get(file_id) else {
            continue;
        };
        let Some(uri) = state.uri_for_file_id(file_id) else {
            continue;
        };

        let mut diagnostics: Vec<lsp_types::Diagnostic> = match state.get_file_diagnostics(file_id) {
            Some(issues) => issues.iter().filter_map(|issue| convert::issue_to_diagnostic(&file, issue)).collect(),
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
