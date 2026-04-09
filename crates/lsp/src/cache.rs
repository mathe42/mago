use std::path::Path;
use std::path::PathBuf;

use foldhash::HashMap;
use serde::Deserialize;
use serde::Serialize;

use mago_codex::metadata::CodebaseMetadata;
use mago_codex::reference::SymbolReferences;
use mago_database::DatabaseReader;
use mago_database::ReadDatabase;
use mago_database::file::FileId;
use mago_database::file::FileType;
use mago_reporting::IssueCollection;

/// Current cache format version. Bump this when the cache format changes
/// (e.g., new fields, different serialization) to invalidate old caches.
const CACHE_VERSION: u64 = 1;

/// The on-disk cache for LSP analysis results.
#[derive(Serialize, Deserialize)]
pub struct LspCache {
    /// Format version — used to invalidate cache on mago upgrades.
    version: u64,
    /// Hash of the mago binary version + config, for invalidation.
    config_hash: u64,
    /// The merged codebase metadata (symbols, types, hierarchy).
    pub codebase: CodebaseMetadata,
    /// The symbol reference graph.
    pub symbol_references: SymbolReferences,
    /// Per-file content hashes — used to detect which files changed.
    pub file_hashes: HashMap<FileId, u64>,
    /// Per-file analysis issues.
    pub file_issues: HashMap<FileId, IssueCollection>,
}

/// The path where the cache file is stored.
pub fn cache_path(workspace: &Path) -> PathBuf {
    workspace.join(".mago").join("lsp-cache.bin")
}

/// Compute a config hash for cache invalidation.
/// Changes when mago version or workspace config changes.
pub fn compute_config_hash(workspace: &Path) -> u64 {
    let mut hasher_input = env!("CARGO_PKG_VERSION").to_string();
    hasher_input.push_str(&workspace.to_string_lossy());
    // Include mago.toml modification time if available.
    let toml_path = workspace.join("mago.toml");
    if let Ok(meta) = std::fs::metadata(&toml_path) {
        if let Ok(modified) = meta.modified() {
            hasher_input.push_str(&format!("{:?}", modified));
        }
    }
    xxhash_rust::xxh3::xxh3_64(hasher_input.as_bytes())
}

/// Compute content hashes for all host files in the database.
pub fn compute_file_hashes(db: &ReadDatabase) -> HashMap<FileId, u64> {
    let mut hashes = HashMap::default();
    for file in db.files() {
        if file.file_type == FileType::Host {
            let hash = xxhash_rust::xxh3::xxh3_64(file.contents.as_bytes());
            hashes.insert(file.id, hash);
        }
    }
    hashes
}

/// Try to load the cache from disk.
///
/// Returns `None` if the cache doesn't exist, is corrupted, or is
/// invalidated (wrong version or config hash).
pub fn load_cache(workspace: &Path) -> Option<LspCache> {
    let path = cache_path(workspace);
    let bytes = std::fs::read(&path).ok()?;

    let config = bincode::config::standard();
    let (cache, _): (LspCache, _) = bincode::serde::decode_from_slice(&bytes, config).ok()?;

    // Validate version.
    if cache.version != CACHE_VERSION {
        tracing::info!("cache version mismatch ({} vs {}), discarding", cache.version, CACHE_VERSION);
        return None;
    }

    // Validate config hash.
    let expected_hash = compute_config_hash(workspace);
    if cache.config_hash != expected_hash {
        tracing::info!("config hash mismatch, discarding cache");
        return None;
    }

    tracing::info!("loaded cache with {} file hashes", cache.file_hashes.len());
    Some(cache)
}

/// Save the cache to disk.
///
/// Creates the `.mago/` directory if it doesn't exist.
pub fn save_cache(
    workspace: &Path,
    codebase: &CodebaseMetadata,
    symbol_references: &SymbolReferences,
    file_hashes: &HashMap<FileId, u64>,
    file_issues: &HashMap<FileId, IssueCollection>,
) {
    let cache = LspCache {
        version: CACHE_VERSION,
        config_hash: compute_config_hash(workspace),
        codebase: codebase.clone(),
        symbol_references: symbol_references.clone(),
        file_hashes: file_hashes.clone(),
        file_issues: file_issues.clone(),
    };

    let config = bincode::config::standard();
    let bytes = match bincode::serde::encode_to_vec(&cache, config) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("failed to serialize cache: {e}");
            return;
        }
    };

    let path = cache_path(workspace);
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::error!("failed to create cache directory: {e}");
            return;
        }
    }

    match std::fs::write(&path, &bytes) {
        Ok(()) => {
            let size_mb = bytes.len() as f64 / (1024.0 * 1024.0);
            tracing::info!("saved cache ({:.1} MB, {} files)", size_mb, cache.file_hashes.len());
        }
        Err(e) => {
            tracing::error!("failed to write cache: {e}");
        }
    }
}

/// Determine which files have changed between the cached hashes and current hashes.
///
/// Returns `(changed, added, removed)` file IDs.
pub fn diff_file_hashes(
    cached: &HashMap<FileId, u64>,
    current: &HashMap<FileId, u64>,
) -> (Vec<FileId>, Vec<FileId>, Vec<FileId>) {
    let mut changed = Vec::new();
    let mut added = Vec::new();
    let mut removed = Vec::new();

    for (&file_id, &current_hash) in current {
        match cached.get(&file_id) {
            Some(&cached_hash) if cached_hash == current_hash => {} // unchanged
            Some(_) => changed.push(file_id),                       // content changed
            None => added.push(file_id),                             // new file
        }
    }

    for &file_id in cached.keys() {
        if !current.contains_key(&file_id) {
            removed.push(file_id); // file deleted
        }
    }

    (changed, added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_id(name: &str) -> FileId {
        FileId::new(name)
    }

    #[test]
    fn test_diff_no_changes() {
        let mut cached = HashMap::default();
        cached.insert(file_id("a.php"), 111);
        cached.insert(file_id("b.php"), 222);

        let current = cached.clone();
        let (changed, added, removed) = diff_file_hashes(&cached, &current);
        assert!(changed.is_empty());
        assert!(added.is_empty());
        assert!(removed.is_empty());
    }

    #[test]
    fn test_diff_file_changed() {
        let mut cached = HashMap::default();
        cached.insert(file_id("a.php"), 111);
        cached.insert(file_id("b.php"), 222);

        let mut current = HashMap::default();
        current.insert(file_id("a.php"), 111);
        current.insert(file_id("b.php"), 999); // changed

        let (changed, added, removed) = diff_file_hashes(&cached, &current);
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0], file_id("b.php"));
        assert!(added.is_empty());
        assert!(removed.is_empty());
    }

    #[test]
    fn test_diff_file_added() {
        let mut cached = HashMap::default();
        cached.insert(file_id("a.php"), 111);

        let mut current = HashMap::default();
        current.insert(file_id("a.php"), 111);
        current.insert(file_id("new.php"), 333);

        let (changed, added, removed) = diff_file_hashes(&cached, &current);
        assert!(changed.is_empty());
        assert_eq!(added.len(), 1);
        assert_eq!(added[0], file_id("new.php"));
        assert!(removed.is_empty());
    }

    #[test]
    fn test_diff_file_removed() {
        let mut cached = HashMap::default();
        cached.insert(file_id("a.php"), 111);
        cached.insert(file_id("old.php"), 222);

        let mut current = HashMap::default();
        current.insert(file_id("a.php"), 111);

        let (changed, added, removed) = diff_file_hashes(&cached, &current);
        assert!(changed.is_empty());
        assert!(added.is_empty());
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], file_id("old.php"));
    }

    #[test]
    fn test_diff_mixed_changes() {
        let mut cached = HashMap::default();
        cached.insert(file_id("keep.php"), 111);
        cached.insert(file_id("modify.php"), 222);
        cached.insert(file_id("delete.php"), 333);

        let mut current = HashMap::default();
        current.insert(file_id("keep.php"), 111);    // unchanged
        current.insert(file_id("modify.php"), 999);   // changed
        current.insert(file_id("create.php"), 444);    // added

        let (changed, added, removed) = diff_file_hashes(&cached, &current);
        assert_eq!(changed.len(), 1);
        assert_eq!(added.len(), 1);
        assert_eq!(removed.len(), 1);
    }

    #[test]
    fn test_cache_roundtrip() {
        let workspace = std::env::temp_dir().join("mago-lsp-test-cache");
        let _ = std::fs::create_dir_all(&workspace);

        let mut file_hashes = HashMap::default();
        file_hashes.insert(file_id("test.php"), 42);

        // Save
        save_cache(
            &workspace,
            &CodebaseMetadata::new(),
            &SymbolReferences::new(),
            &file_hashes,
            &HashMap::default(),
        );

        // Load
        let loaded = load_cache(&workspace);
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.file_hashes.len(), 1);
        assert_eq!(*loaded.file_hashes.get(&file_id("test.php")).unwrap(), 42);

        // Cleanup
        let _ = std::fs::remove_dir_all(workspace.join(".mago"));
    }

    #[test]
    fn test_cache_invalidated_on_missing_file() {
        let workspace = std::env::temp_dir().join("mago-lsp-test-cache-missing");
        let loaded = load_cache(&workspace);
        assert!(loaded.is_none());
    }
}
