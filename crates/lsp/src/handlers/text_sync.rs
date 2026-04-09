use lsp_server::Connection;
use lsp_types::DidChangeTextDocumentParams;
use lsp_types::DidCloseTextDocumentParams;
use lsp_types::DidOpenTextDocumentParams;
use lsp_types::notification::Notification;

use crate::handlers::diagnostics::publish_diagnostics;
use crate::state::LspState;

/// Handle `textDocument/didOpen`.
pub fn handle_did_open(state: &mut LspState, connection: &Connection, params: DidOpenTextDocumentParams) {
    let uri = params.text_document.uri;
    let version = params.text_document.version;
    let content = params.text_document.text;

    tracing::debug!("didOpen: {}", uri.as_str());
    state.open_document(uri.clone(), version, content);

    // Run analysis and publish diagnostics.
    if let Some(file_id) = state.file_id_for_uri(&uri) {
        let analyzed = state.analyze_changes(&[file_id]);
        publish_diagnostics(state, connection, &analyzed);
    }
}

/// Handle `textDocument/didChange` (full text sync).
pub fn handle_did_change(state: &mut LspState, connection: &Connection, params: DidChangeTextDocumentParams) {
    let uri = params.text_document.uri;
    let version = params.text_document.version;

    // With full text sync, there's exactly one content change containing the whole document.
    let Some(change) = params.content_changes.into_iter().next() else {
        return;
    };

    tracing::debug!("didChange: {} (v{version})", uri.as_str());
    state.change_document(&uri, version, change.text);

    // Run incremental analysis and publish diagnostics.
    if let Some(file_id) = state.file_id_for_uri(&uri) {
        let analyzed = state.analyze_changes(&[file_id]);
        publish_diagnostics(state, connection, &analyzed);
    }
}

/// Handle `textDocument/didClose`.
pub fn handle_did_close(state: &mut LspState, connection: &Connection, params: DidCloseTextDocumentParams) {
    let uri = params.text_document.uri;
    tracing::debug!("didClose: {}", uri.as_str());
    state.close_document(&uri);

    // Clear diagnostics for the closed file.
    let params = lsp_types::PublishDiagnosticsParams {
        uri,
        diagnostics: vec![],
        version: None,
    };

    let notification = lsp_server::Notification::new(
        lsp_types::notification::PublishDiagnostics::METHOD.to_string(),
        params,
    );

    if let Err(e) = connection.sender.send(lsp_server::Message::Notification(notification)) {
        tracing::error!("failed to send clear diagnostics: {e}");
    }
}
