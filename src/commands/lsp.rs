use std::borrow::Cow;
use std::process::ExitCode;

use clap::ColorChoice;
use clap::Parser;

use mago_database::DatabaseConfiguration;
use mago_database::DatabaseReader;
use mago_database::exclusion::Exclusion;
use mago_database::file::FileType;
use mago_database::loader::DatabaseLoader;
use mago_prelude::Prelude;

use crate::config::Configuration;
use crate::consts::PRELUDE_BYTES;
use crate::error::Error;
use crate::utils::create_orchestrator;

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
    pub fn execute(self, configuration: Configuration, color_choice: ColorChoice) -> Result<ExitCode, Error> {
        let workspace = configuration.source.workspace.clone();

        // 1. Load prelude (PHP built-in stubs).
        let Prelude { database: prelude_db, metadata, symbol_references } = if self.no_stubs {
            Prelude::default()
        } else {
            Prelude::decode(PRELUDE_BYTES).expect("Failed to decode embedded prelude")
        };

        // 2. Build database config from mago.toml source settings.
        let mut excludes: Vec<Exclusion<'static>> = Vec::new();
        for pattern in &configuration.source.excludes {
            if pattern.contains('*') {
                excludes.push(Exclusion::Pattern(Cow::Owned(pattern.clone())));
            } else {
                let path = if std::path::Path::new(pattern).is_absolute() {
                    std::path::PathBuf::from(pattern)
                } else {
                    workspace.join(pattern)
                };
                excludes.push(Exclusion::Path(Cow::Owned(path.canonicalize().unwrap_or(path))));
            }
        }
        for pattern in &configuration.analyzer.excludes {
            excludes.push(Exclusion::Pattern(Cow::Owned(pattern.clone())));
        }

        let db_config = DatabaseConfiguration {
            workspace: Cow::Owned(workspace.clone()),
            paths: configuration.source.paths.iter().map(|s| Cow::Owned(s.clone())).collect(),
            includes: configuration.source.includes.iter().map(|s| Cow::Owned(s.clone())).collect(),
            excludes,
            extensions: configuration.source.extensions.iter().map(|s| Cow::Owned(s.clone())).collect(),
            glob: configuration.source.glob.to_database_settings(),
        };

        let loader = DatabaseLoader::new(db_config).with_database(prelude_db);
        let database = loader.load()?;

        let host_count = database.files().filter(|f| f.file_type == FileType::Host).count();
        tracing::info!("loaded {host_count} host files for LSP");

        // 3. Create analysis service with real config.
        let orchestrator = create_orchestrator(&configuration, color_choice, false, false, false);
        let analysis_service =
            orchestrator.get_incremental_analysis_service(database.read_only(), metadata, symbol_references);
        let parser_settings = orchestrator.config.parser_settings;
        let stack_size = configuration.stack_size;

        // 4. Start LSP on a thread with a large stack (needed for parsing very large files).
        //    The default thread stack (1-8 MB) is too small for files like data.inc.php (46k lines).
        let lsp_config = mago_lsp::LspConfig {
            workspace,
            database,
            analysis_service,
            parser_settings,
        };

        let lsp_stack_size = stack_size.max(64 * 1024 * 1024); // at least 64 MB
        let builder = std::thread::Builder::new()
            .name("lsp-main".to_string())
            .stack_size(lsp_stack_size);

        let handle = builder
            .spawn(move || mago_lsp::run_server(lsp_config))
            .map_err(|e| Error::Lsp(format!("failed to spawn LSP thread: {e}")))?;

        handle
            .join()
            .map_err(|_| Error::Lsp("LSP thread panicked".to_string()))?
            .map_err(|e| Error::Lsp(e.to_string()))?;

        Ok(ExitCode::SUCCESS)
    }
}
