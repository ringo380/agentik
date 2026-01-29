//! Cache persistence and file watching for incremental updates.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher, Event, EventKind};
use tracing::{debug, info, warn};

use crate::types::RepoMap;

/// Cache file name.
const CACHE_FILE: &str = "repomap.json";

/// Directory for storing cache files.
const CACHE_DIR: &str = ".agentik";

/// Error type for cache operations.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("file watcher error: {0}")]
    Watcher(#[from] notify::Error),

    #[error("cache version mismatch: expected {expected}, found {found}")]
    VersionMismatch { expected: u32, found: u32 },
}

/// Pending file updates tracked by the file watcher.
#[derive(Debug, Default)]
pub struct PendingUpdates {
    /// Files that were created or modified
    pub modified: HashSet<PathBuf>,
    /// Files that were deleted
    pub deleted: HashSet<PathBuf>,
}

impl PendingUpdates {
    /// Create empty pending updates.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if there are any pending updates.
    pub fn has_updates(&self) -> bool {
        !self.modified.is_empty() || !self.deleted.is_empty()
    }

    /// Clear all pending updates.
    pub fn clear(&mut self) {
        self.modified.clear();
        self.deleted.clear();
    }

    /// Take all pending updates, leaving empty.
    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }
}

/// Repository map cache for persistence and incremental updates.
pub struct RepoMapCache {
    /// Root directory of the repository
    root: PathBuf,
    /// File watcher (if active)
    watcher: Option<RecommendedWatcher>,
    /// Pending file updates
    pending: Arc<Mutex<PendingUpdates>>,
    /// Supported file extensions to watch
    extensions: HashSet<String>,
}

impl RepoMapCache {
    /// Create a new cache for a repository.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let extensions: HashSet<String> = [
            "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs",
            "py", "pyi", "go", "java",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        Self {
            root: root.into(),
            watcher: None,
            pending: Arc::new(Mutex::new(PendingUpdates::new())),
            extensions,
        }
    }

    /// Get the cache file path.
    pub fn cache_path(&self) -> PathBuf {
        self.root.join(CACHE_DIR).join(CACHE_FILE)
    }

    /// Load the cached repo map from disk.
    pub fn load(&self) -> Result<Option<RepoMap>, CacheError> {
        let path = self.cache_path();

        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&path)?;
        let map: RepoMap = serde_json::from_str(&content)?;

        // Check version compatibility
        if map.version != RepoMap::VERSION {
            return Err(CacheError::VersionMismatch {
                expected: RepoMap::VERSION,
                found: map.version,
            });
        }

        info!("Loaded repo map from cache: {} files", map.file_count());
        Ok(Some(map))
    }

    /// Save the repo map to disk.
    pub fn save(&self, map: &RepoMap) -> Result<(), CacheError> {
        let path = self.cache_path();

        // Ensure directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(map)?;
        std::fs::write(&path, content)?;

        info!("Saved repo map to cache: {} files", map.file_count());
        Ok(())
    }

    /// Check if a file needs to be updated based on modification time.
    pub fn needs_update(&self, map: &RepoMap, path: &Path) -> bool {
        // Get file metadata
        let metadata = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => return true, // File doesn't exist or can't be read
        };

        let current_mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);

        // Check if we have this file in the cache
        let relative = self.make_relative(path);
        match map.get_file(&relative) {
            Some(info) => info.mtime < current_mtime,
            None => true, // File not in cache
        }
    }

    /// Get files that need to be updated.
    pub fn files_needing_update(&self, map: &RepoMap, files: &[PathBuf]) -> Vec<PathBuf> {
        files
            .iter()
            .filter(|f| self.needs_update(map, f))
            .cloned()
            .collect()
    }

    /// Start watching the repository for file changes.
    pub fn start_watching(&mut self) -> Result<(), CacheError> {
        if self.watcher.is_some() {
            return Ok(()); // Already watching
        }

        let pending = Arc::clone(&self.pending);
        let extensions = self.extensions.clone();
        let root = self.root.clone();

        let mut watcher = RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| {
                match result {
                    Ok(event) => {
                        Self::handle_event(&event, &pending, &extensions, &root);
                    }
                    Err(e) => {
                        warn!("File watcher error: {}", e);
                    }
                }
            },
            Config::default(),
        )?;

        watcher.watch(&self.root, RecursiveMode::Recursive)?;
        self.watcher = Some(watcher);

        info!("Started watching repository for changes: {:?}", self.root);
        Ok(())
    }

    /// Stop watching the repository.
    pub fn stop_watching(&mut self) {
        if let Some(mut watcher) = self.watcher.take() {
            let _ = watcher.unwatch(&self.root);
            info!("Stopped watching repository");
        }
    }

    /// Check if watching is active.
    pub fn is_watching(&self) -> bool {
        self.watcher.is_some()
    }

    /// Get pending updates.
    pub fn pending_updates(&self) -> PendingUpdates {
        let mut pending = self.pending.lock().unwrap();
        pending.take()
    }

    /// Check if there are pending updates.
    pub fn has_pending_updates(&self) -> bool {
        let pending = self.pending.lock().unwrap();
        pending.has_updates()
    }

    /// Handle a file system event.
    fn handle_event(
        event: &Event,
        pending: &Arc<Mutex<PendingUpdates>>,
        extensions: &HashSet<String>,
        root: &Path,
    ) {
        let paths: Vec<_> = event
            .paths
            .iter()
            .filter(|p| Self::should_track_file(p, extensions))
            .map(|p| Self::make_relative_static(root, p))
            .collect();

        if paths.is_empty() {
            return;
        }

        let mut pending = pending.lock().unwrap();

        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                for path in paths {
                    debug!("File modified: {:?}", path);
                    pending.deleted.remove(&path);
                    pending.modified.insert(path);
                }
            }
            EventKind::Remove(_) => {
                for path in paths {
                    debug!("File deleted: {:?}", path);
                    pending.modified.remove(&path);
                    pending.deleted.insert(path);
                }
            }
            _ => {}
        }
    }

    /// Check if a file should be tracked based on its extension.
    fn should_track_file(path: &Path, extensions: &HashSet<String>) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| extensions.contains(e))
            .unwrap_or(false)
    }

    /// Make a path relative to root.
    fn make_relative(&self, path: &Path) -> PathBuf {
        Self::make_relative_static(&self.root, path)
    }

    fn make_relative_static(root: &Path, path: &Path) -> PathBuf {
        path.strip_prefix(root)
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| path.to_path_buf())
    }

    /// Delete the cache file.
    pub fn clear(&self) -> Result<(), CacheError> {
        let path = self.cache_path();
        if path.exists() {
            std::fs::remove_file(&path)?;
            info!("Cleared repo map cache");
        }
        Ok(())
    }
}

impl Drop for RepoMapCache {
    fn drop(&mut self) {
        self.stop_watching();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_cache_path() {
        let cache = RepoMapCache::new("/project");
        assert_eq!(cache.cache_path(), PathBuf::from("/project/.agentik/repomap.json"));
    }

    #[test]
    fn test_save_and_load() {
        let temp = TempDir::new().unwrap();
        let cache = RepoMapCache::new(temp.path());

        // Create and save a repo map
        let mut map = RepoMap::new(temp.path());
        map.ranks.insert(PathBuf::from("test.rs"), 0.5);

        cache.save(&map).unwrap();
        assert!(cache.cache_path().exists());

        // Load it back
        let loaded = cache.load().unwrap().unwrap();
        assert_eq!(loaded.ranks.get(&PathBuf::from("test.rs")), Some(&0.5));
    }

    #[test]
    fn test_load_nonexistent() {
        let temp = TempDir::new().unwrap();
        let cache = RepoMapCache::new(temp.path());

        let result = cache.load().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_clear() {
        let temp = TempDir::new().unwrap();
        let cache = RepoMapCache::new(temp.path());

        // Create cache
        let map = RepoMap::new(temp.path());
        cache.save(&map).unwrap();
        assert!(cache.cache_path().exists());

        // Clear it
        cache.clear().unwrap();
        assert!(!cache.cache_path().exists());
    }

    #[test]
    fn test_pending_updates() {
        let mut pending = PendingUpdates::new();
        assert!(!pending.has_updates());

        pending.modified.insert(PathBuf::from("test.rs"));
        assert!(pending.has_updates());

        let taken = pending.take();
        assert!(taken.has_updates());
        assert!(!pending.has_updates());
    }

    #[test]
    fn test_should_track_file() {
        let extensions: HashSet<String> = ["rs", "ts", "py"].iter().map(|s| s.to_string()).collect();

        assert!(RepoMapCache::should_track_file(Path::new("test.rs"), &extensions));
        assert!(RepoMapCache::should_track_file(Path::new("test.ts"), &extensions));
        assert!(RepoMapCache::should_track_file(Path::new("test.py"), &extensions));
        assert!(!RepoMapCache::should_track_file(Path::new("test.txt"), &extensions));
        assert!(!RepoMapCache::should_track_file(Path::new("test"), &extensions));
    }
}
