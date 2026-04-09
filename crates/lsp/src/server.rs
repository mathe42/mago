use std::path::PathBuf;

use lsp_server::Connection;
use lsp_server::Message;
use lsp_types::InitializeParams;
use lsp_types::ServerCapabilities;
use lsp_types::TextDocumentSyncCapability;
use lsp_types::TextDocumentSyncKind;
use lsp_types::notification::Notification;
use lsp_types::request::Request;

use crate::LspConfig;
use crate::error::ServerError;
use crate::handlers;
use crate::state::LspState;

/// Run the LSP server over stdio.
///
/// This blocks until the client sends a shutdown request.
pub fn run(config: LspConfig) -> Result<(), ServerError> {
    let (connection, io_threads) = Connection::stdio();

    // Phase 1: Initialize handshake.
    let server_capabilities = build_capabilities();
    let capabilities_json = serde_json::to_value(server_capabilities)?;
    let init_params = connection.initialize(capabilities_json)?;
    let _init_params: InitializeParams = serde_json::from_value(init_params)?;

    tracing::info!("workspace root: {}", config.workspace.display());

    // Phase 2: Initialize state (runs initial analysis).
    let mut state = LspState::initialize(config)?;
    tracing::info!("LSP server initialized, entering main loop");

    // Phase 3: Main message loop.
    main_loop(&mut state, &connection)?;

    // Phase 4: Clean shutdown.
    io_threads.join()?;
    tracing::info!("LSP server shut down");

    Ok(())
}

fn build_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(lsp_types::CompletionOptions {
            trigger_characters: Some(vec![
                ">".to_string(), // for ->
                ":".to_string(), // for ::
                "$".to_string(), // for variables
                "\\".to_string(), // for namespace separators
            ]),
            ..Default::default()
        }),
        definition_provider: Some(lsp_types::OneOf::Left(true)),
        references_provider: Some(lsp_types::OneOf::Left(true)),
        rename_provider: Some(lsp_types::OneOf::Left(true)),
        hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
        document_symbol_provider: Some(lsp_types::OneOf::Left(true)),
        document_formatting_provider: Some(lsp_types::OneOf::Left(true)),
        code_action_provider: Some(lsp_types::CodeActionProviderCapability::Simple(true)),
        ..ServerCapabilities::default()
    }
}

fn resolve_workspace_root(params: &InitializeParams, fallback: PathBuf) -> PathBuf {
    // Try workspace folders first (LSP 3.x).
    if let Some(ref folders) = params.workspace_folders {
        if let Some(folder) = folders.first() {
            if let Some(path) = crate::convert::uri_to_path(&folder.uri) {
                return path;
            }
        }
    }

    // Fall back to the deprecated root_uri.
    #[allow(deprecated)]
    if let Some(ref root_uri) = params.root_uri {
        if let Some(path) = crate::convert::uri_to_path(root_uri) {
            return path;
        }
    }

    fallback
}

fn main_loop(state: &mut LspState, connection: &Connection) -> Result<(), ServerError> {
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }

                handle_request(state, connection, req)?;
            }
            Message::Notification(not) => {
                handle_notification(state, connection, not)?;
            }
            Message::Response(_resp) => {
                // We don't send requests to the client yet, so ignore responses.
            }
        }
    }

    Ok(())
}

fn handle_request(
    state: &mut LspState,
    connection: &Connection,
    req: lsp_server::Request,
) -> Result<(), ServerError> {
    let method = req.method.as_str();

    match method {
        lsp_types::request::Completion::METHOD => {
            let (id, params) = req.extract::<lsp_types::CompletionParams>(
                lsp_types::request::Completion::METHOD,
            )?;

            let result = handlers::completion::handle_completion(state, params)?;
            let response = lsp_server::Response::new_ok(id, result);
            connection.sender.send(Message::Response(response)).ok();
        }
        lsp_types::request::GotoDefinition::METHOD => {
            let (id, params) = req.extract::<lsp_types::GotoDefinitionParams>(
                lsp_types::request::GotoDefinition::METHOD,
            )?;

            let result = handlers::definition::handle_goto_definition(state, params)?;
            let response = lsp_server::Response::new_ok(id, result);
            connection.sender.send(Message::Response(response)).ok();
        }
        lsp_types::request::HoverRequest::METHOD => {
            let (id, params) = req.extract::<lsp_types::HoverParams>(
                lsp_types::request::HoverRequest::METHOD,
            )?;

            let result = handlers::hover::handle_hover(state, params)?;
            let response = lsp_server::Response::new_ok(id, result);
            connection.sender.send(Message::Response(response)).ok();
        }
        lsp_types::request::DocumentSymbolRequest::METHOD => {
            let (id, params) = req.extract::<lsp_types::DocumentSymbolParams>(
                lsp_types::request::DocumentSymbolRequest::METHOD,
            )?;

            let result = handlers::document_symbols::handle_document_symbols(state, params)?;
            let response = lsp_server::Response::new_ok(id, result);
            connection.sender.send(Message::Response(response)).ok();
        }
        lsp_types::request::References::METHOD => {
            let (id, params) = req.extract::<lsp_types::ReferenceParams>(
                lsp_types::request::References::METHOD,
            )?;

            let result = handlers::references::handle_references(state, params)?;
            let response = lsp_server::Response::new_ok(id, result);
            connection.sender.send(Message::Response(response)).ok();
        }
        lsp_types::request::Rename::METHOD => {
            let (id, params) = req.extract::<lsp_types::RenameParams>(
                lsp_types::request::Rename::METHOD,
            )?;

            let result = handlers::rename::handle_rename(state, params)?;
            let response = lsp_server::Response::new_ok(id, result);
            connection.sender.send(Message::Response(response)).ok();
        }
        lsp_types::request::CodeActionRequest::METHOD => {
            let (id, params) = req.extract::<lsp_types::CodeActionParams>(
                lsp_types::request::CodeActionRequest::METHOD,
            )?;

            let result = handlers::code_action::handle_code_action(state, params)?;
            let response = lsp_server::Response::new_ok(id, result);
            connection.sender.send(Message::Response(response)).ok();
        }
        lsp_types::request::Formatting::METHOD => {
            let (id, params) = req.extract::<lsp_types::DocumentFormattingParams>(
                lsp_types::request::Formatting::METHOD,
            )?;

            let result = handlers::formatting::handle_formatting(state, params)?;
            let response = lsp_server::Response::new_ok(id, result);
            connection.sender.send(Message::Response(response)).ok();
        }
        _ => {
            tracing::debug!("unhandled request: {method}");
            // Respond with MethodNotFound.
            let response = lsp_server::Response::new_err(
                req.id,
                lsp_server::ErrorCode::MethodNotFound as i32,
                format!("method not found: {method}"),
            );
            connection.sender.send(Message::Response(response)).ok();
        }
    }

    Ok(())
}

fn handle_notification(
    state: &mut LspState,
    connection: &Connection,
    not: lsp_server::Notification,
) -> Result<(), ServerError> {
    let method = not.method.as_str();

    match method {
        lsp_types::notification::DidOpenTextDocument::METHOD => {
            let params: lsp_types::DidOpenTextDocumentParams = serde_json::from_value(not.params)?;
            handlers::text_sync::handle_did_open(state, connection, params);
        }
        lsp_types::notification::DidChangeTextDocument::METHOD => {
            let params: lsp_types::DidChangeTextDocumentParams = serde_json::from_value(not.params)?;
            handlers::text_sync::handle_did_change(state, connection, params);
        }
        lsp_types::notification::DidCloseTextDocument::METHOD => {
            let params: lsp_types::DidCloseTextDocumentParams = serde_json::from_value(not.params)?;
            handlers::text_sync::handle_did_close(state, connection, params);
        }
        _ => {
            tracing::debug!("unhandled notification: {method}");
        }
    }

    Ok(())
}
