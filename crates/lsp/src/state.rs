use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use foldhash::HashMap;
use lsp_types::Uri;

use mago_codex::metadata::CodebaseMetadata;
use mago_database::Database;
use mago_database::DatabaseConfiguration;
use mago_database::DatabaseReader;
use mago_database::GlobSettings;
use mago_database::ReadDatabase;
use mago_database::file::File;
use mago_database::file::FileId;
use mago_database::file::FileType;
use mago_database::loader::DatabaseLoader;
use mago_orchestrator::Orchestrator;
use mago_orchestrator::OrchestratorConfiguration;
use mago_orchestrator::service::incremental_analysis::IncrementalAnalysisService;
use mago_php_version::PHPVersion;
use mago_prelude::Prelude;
use mago_reporting::IssueCollection;
use mago_syntax::settings::ParserSettings;

use crate::convert;
use crate::error::ServerError;

/// Tracks the state of a document open in the editor.
#[derive(Debug)]
pub struct OpenDocument {
    pub uri: Uri,
    pub file_id: FileId,
    pub version: i32,
    pub content: String,
}

/// Central mutable state for the language server.
///
/// Owns the file database, analysis service, and open document tracking.
/// All mutations happen on the main thread; analysis results are read
/// from the `IncrementalAnalysisService`.
pub struct LspState {
    /// Workspace root directory.
    pub workspace: PathBuf,
    /// The mutable file database.
    database: Database<'static>,
    /// The incremental analysis service.
    analysis_service: IncrementalAnalysisService,
    /// Currently open documents keyed by URI string for reliable lookup.
    open_documents: HashMap<String, OpenDocument>,
    /// URI string → FileId mapping for quick lookups.
    uri_to_file_id: HashMap<String, FileId>,
    /// FileId → URI reverse mapping.
    file_id_to_uri: HashMap<FileId, Uri>,
    /// Parser settings.
    parser_settings: ParserSettings,
}

impl LspState {
    /// Initialize the LSP state for the given workspace.
    ///
    /// The `prelude` provides pre-compiled metadata for PHP built-in symbols.
    pub fn initialize(workspace: PathBuf, prelude: Prelude) -> Result<Self, ServerError> {
        tracing::info!("initializing LSP state for workspace: {}", workspace.display());

        let prelude_metadata = prelude.metadata;
        let prelude_refs = prelude.symbol_references;
        let prelude_database = prelude.database;

        // Build the database configuration for the workspace.
        let db_config = DatabaseConfiguration {
            workspace: Cow::Owned(workspace.clone()),
            paths: vec![Cow::Borrowed(".")],
            includes: vec![],
            excludes: vec![],
            extensions: vec![Cow::Borrowed("php")],
            glob: GlobSettings::default(),
        };

        let loader = DatabaseLoader::new(db_config).with_database(prelude_database);
        let database = loader.load().map_err(|e| ServerError::Message(format!("failed to load database: {e}")))?;

        let parser_settings = ParserSettings::default();

        // Create the orchestrator to get the analysis service.
        let orchestrator = Orchestrator::new(OrchestratorConfiguration {
            php_version: PHPVersion::default(),
            paths: vec![".".to_string()],
            includes: vec![],
            excludes: vec![],
            extensions: vec!["php"],
            glob: GlobSettings::default(),
            parser_settings,
            analyzer_settings: Default::default(),
            linter_settings: Default::default(),
            guard_settings: Default::default(),
            formatter_settings: Default::default(),
            disable_default_analyzer_plugins: false,
            analyzer_plugins: vec![],
            use_progress_bars: false,
            use_colors: false,
        });

        let read_db = database.read_only();

        // Build URI mappings from the database.
        let mut uri_to_file_id = HashMap::default();
        let mut file_id_to_uri = HashMap::default();
        for file in read_db.files() {
            if let Some(ref path) = file.path {
                let uri = convert::path_to_uri(path);
                let key = uri.as_str().to_string();
                uri_to_file_id.insert(key, file.id);
                file_id_to_uri.insert(file.id, uri);
            }
        }

        // Create the incremental analysis service.
        let mut analysis_service =
            orchestrator.get_incremental_analysis_service(read_db, prelude_metadata, prelude_refs);

        // Run the initial full analysis.
        tracing::info!("running initial analysis...");
        match analysis_service.analyze() {
            Ok(_result) => {
                tracing::info!(
                    "initial analysis complete, tracking {} files",
                    analysis_service.tracked_file_count()
                );
            }
            Err(e) => {
                tracing::error!("initial analysis failed: {e}");
            }
        }

        Ok(Self {
            workspace,
            database,
            analysis_service,
            open_documents: HashMap::default(),
            uri_to_file_id,
            file_id_to_uri,
            parser_settings,
        })
    }

    /// Handle a document being opened in the editor.
    pub fn open_document(&mut self, uri: Uri, version: i32, content: String) {
        let file_id = self.ensure_file_id(&uri, &content);
        let key = uri.as_str().to_string();
        self.open_documents.insert(
            key,
            OpenDocument { uri, file_id, version, content },
        );
    }

    /// Handle a document being changed in the editor (full sync).
    pub fn change_document(&mut self, uri: &Uri, version: i32, content: String) {
        let key = uri.as_str();
        if let Some(doc) = self.open_documents.get_mut(key) {
            doc.version = version;
            doc.content = content.clone();
            self.database.update(doc.file_id, Cow::Owned(content));
        }
    }

    /// Handle a document being closed in the editor.
    pub fn close_document(&mut self, uri: &Uri) {
        self.open_documents.remove(uri.as_str());
    }

    /// Run incremental analysis on changed files and return their FileIds.
    pub fn analyze_changes(&mut self, file_ids: &[FileId]) -> Vec<FileId> {
        let read_db = self.database.read_only();
        self.analysis_service.update_database(read_db);

        match self.analysis_service.analyze_incremental(Some(file_ids)) {
            Ok(_) => file_ids.to_vec(),
            Err(e) => {
                tracing::error!("incremental analysis failed: {e}");
                vec![]
            }
        }
    }

    /// Get diagnostics for a specific file from the last analysis run.
    pub fn get_file_diagnostics(&self, file_id: &FileId) -> Option<&IssueCollection> {
        self.analysis_service.get_file_diagnostics(file_id)
    }

    /// Get the current codebase metadata.
    pub fn codebase(&self) -> &CodebaseMetadata {
        self.analysis_service.codebase()
    }

    /// Get the file database (read-only snapshot).
    pub fn database(&self) -> &ReadDatabase {
        self.analysis_service.database()
    }

    /// Look up a file by its URI.
    pub fn file_id_for_uri(&self, uri: &Uri) -> Option<FileId> {
        self.uri_to_file_id.get(uri.as_str()).copied()
    }

    /// Look up a URI by FileId.
    pub fn uri_for_file_id(&self, file_id: &FileId) -> Option<&Uri> {
        self.file_id_to_uri.get(file_id)
    }

    /// Get a file from the database by its FileId.
    pub fn get_file(&self, file_id: &FileId) -> Option<Arc<File>> {
        self.analysis_service.database().get(file_id).ok()
    }

    /// Ensure a FileId exists for the given URI, creating one if needed.
    fn ensure_file_id(&mut self, uri: &Uri, content: &str) -> FileId {
        let key = uri.as_str();
        if let Some(&id) = self.uri_to_file_id.get(key) {
            return id;
        }

        // Derive the file name from the URI relative to the workspace.
        let path = convert::uri_to_path(uri).unwrap_or_else(|| PathBuf::from(uri.path().as_str()));
        let name = path
            .strip_prefix(&self.workspace)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        let file = File::new(
            Cow::Owned(name),
            FileType::Host,
            Some(path),
            Cow::Owned(content.to_string()),
        );
        let file_id = file.id;
        self.database.add(file);
        self.uri_to_file_id.insert(key.to_string(), file_id);
        self.file_id_to_uri.insert(file_id, uri.clone());
        file_id
    }
}
