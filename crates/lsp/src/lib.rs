use std::path::PathBuf;

use error::ServerError;
use mago_prelude::Prelude;

pub mod convert;
pub mod error;
pub mod handlers;
pub mod navigate;
pub mod server;
pub mod state;

/// Starts the LSP server over stdio.
///
/// This is the main entry point called by the `mago lsp` CLI command.
/// It blocks until the client sends a shutdown request.
///
/// The `prelude` provides pre-compiled metadata for PHP built-in symbols.
pub fn run_server(workspace: PathBuf, prelude: Prelude) -> Result<(), ServerError> {
    server::run(workspace, prelude)
}
