use std::path::PathBuf;

use error::ServerError;
use mago_database::Database;
use mago_orchestrator::service::incremental_analysis::IncrementalAnalysisService;
use mago_syntax::settings::ParserSettings;

pub mod cache;
pub mod convert;
pub mod error;
pub mod handlers;
pub mod navigate;
pub mod server;
pub mod state;

/// Configuration for the LSP server, built from the project's `mago.toml`.
pub struct LspConfig {
    pub workspace: PathBuf,
    pub database: Database<'static>,
    pub analysis_service: IncrementalAnalysisService,
    pub parser_settings: ParserSettings,
}

/// Starts the LSP server over stdio.
///
/// This is the main entry point called by the `mago lsp` CLI command.
/// It blocks until the client sends a shutdown request.
pub fn run_server(config: LspConfig) -> Result<(), ServerError> {
    server::run(config)
}
