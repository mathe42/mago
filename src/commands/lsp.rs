use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use mago_prelude::Prelude;

use crate::config::Configuration;
use crate::consts::PRELUDE_BYTES;
use crate::error::Error;

/// Start the Mago Language Server Protocol (LSP) server.
///
/// The LSP server communicates over stdio and provides diagnostics,
/// formatting, and other language features to editors and IDEs.
///
/// **Usage**: `mago lsp`
#[derive(Parser, Debug)]
#[command(name = "lsp")]
pub struct LspCommand {
    /// Disable built-in PHP and library stubs.
    #[arg(long, default_value_t = false)]
    pub no_stubs: bool,
}

impl LspCommand {
    pub fn execute(self, configuration: Configuration) -> Result<ExitCode, Error> {
        let workspace = configuration.source.workspace.clone();

        let prelude = if self.no_stubs {
            Prelude::default()
        } else {
            Prelude::decode(PRELUDE_BYTES).expect("Failed to decode embedded prelude")
        };

        mago_lsp::run_server(workspace, prelude).map_err(|e| Error::Lsp(e.to_string()))?;

        Ok(ExitCode::SUCCESS)
    }
}
