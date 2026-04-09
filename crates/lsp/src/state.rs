use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use foldhash::HashMap;
use lsp_types::Uri;

use mago_codex::metadata::CodebaseMetadata;
use mago_database::Database;
use mago_database::DatabaseReader;
use mago_database::ReadDatabase;
use mago_database::file::File;
use mago_database::file::FileId;
use mago_database::file::FileType;
use mago_orchestrator::service::incremental_analysis::IncrementalAnalysisService;
use mago_reporting::IssueCollection;
use mago_syntax::settings::ParserSettings;

use crate::LspConfig;
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
pub struct LspState {
    pub workspace: PathBuf,
    database: Database<'static>,
    analysis_service: IncrementalAnalysisService,
    open_documents: HashMap<String, OpenDocument>,
    uri_to_file_id: HashMap<String, FileId>,
    file_id_to_uri: HashMap<FileId, Uri>,
    parser_settings: ParserSettings,
}

impl LspState {
    /// Initialize the LSP state from a pre-built config.
    ///
    /// The `LspConfig` is constructed by the CLI command using the Orchestrator,
    /// ensuring the database and analysis service use the real `mago.toml` configuration.
    pub fn initialize(config: LspConfig) -> Result<Self, ServerError> {
        let LspConfig { workspace, database, mut analysis_service, parser_settings } = config;

        tracing::info!("initializing LSP state for workspace: {}", workspace.display());

        let host_count = database.files().filter(|f| f.file_type == FileType::Host).count();
        tracing::info!("loaded {host_count} host files");

        // Build URI mappings from the database.
        let read_db = database.read_only();
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

        // The analysis service already has the database snapshot; run initial analysis.
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

    pub fn open_document(&mut self, uri: Uri, version: i32, content: String) {
        let file_id = self.ensure_file_id(&uri, &content);
        let key = uri.as_str().to_string();
        self.open_documents.insert(key, OpenDocument { uri, file_id, version, content });
    }

    pub fn change_document(&mut self, uri: &Uri, version: i32, content: String) {
        let key = uri.as_str();
        if let Some(doc) = self.open_documents.get_mut(key) {
            doc.version = version;
            doc.content = content.clone();
            self.database.update(doc.file_id, Cow::Owned(content));
        }
    }

    pub fn close_document(&mut self, uri: &Uri) {
        self.open_documents.remove(uri.as_str());
    }

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

    pub fn get_file_diagnostics(&self, file_id: &FileId) -> Option<&IssueCollection> {
        self.analysis_service.get_file_diagnostics(file_id)
    }

    pub fn codebase(&self) -> &CodebaseMetadata {
        self.analysis_service.codebase()
    }

    pub fn database(&self) -> &ReadDatabase {
        self.analysis_service.database()
    }

    pub fn file_id_for_uri(&self, uri: &Uri) -> Option<FileId> {
        self.uri_to_file_id.get(uri.as_str()).copied()
    }

    pub fn uri_for_file_id(&self, file_id: &FileId) -> Option<&Uri> {
        self.file_id_to_uri.get(file_id)
    }

    pub fn get_file(&self, file_id: &FileId) -> Option<Arc<File>> {
        self.analysis_service.database().get(file_id).ok()
    }

    fn ensure_file_id(&mut self, uri: &Uri, content: &str) -> FileId {
        let key = uri.as_str();
        if let Some(&id) = self.uri_to_file_id.get(key) {
            return id;
        }

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
