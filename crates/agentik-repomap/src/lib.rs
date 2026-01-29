//! # agentik-repomap
//!
//! Repository mapping and code analysis for Agentik.
//!
//! This crate provides:
//! - Tree-sitter based multi-language parsing
//! - Symbol extraction (functions, classes, types)
//! - Dependency graph construction
//! - PageRank-based file ranking
//! - Context-aware serialization
//!
//! ## Quick Start
//!
//! ```ignore
//! use agentik_repomap::RepoMapBuilder;
//!
//! // Build a repo map for a project
//! let builder = RepoMapBuilder::new("/path/to/project");
//! let repo_map = builder.build().await?;
//!
//! // Get the most important files
//! let top_files = repo_map.files_by_rank();
//!
//! // Serialize for prompt injection
//! let prompt_text = builder.serialize_for_prompt(2000)?;
//! ```

pub mod cache;
pub mod graph;
pub mod parser;
pub mod ranking;
pub mod serializer;
pub mod types;

pub use cache::{CacheError, PendingUpdates, RepoMapCache};
pub use graph::DependencyGraph;
pub use parser::{ParseError, TreeSitterParser};
pub use ranking::{PageRankConfig, PageRankScorer};
pub use serializer::{RepoMapSerializer, SerializeConfig};
pub use types::{FileInfo, Import, Language, RepoMap, Symbol, SymbolKind};

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ignore::WalkBuilder;
use tracing::{debug, info, warn};

/// Error type for repo map operations.
#[derive(Debug, thiserror::Error)]
pub enum RepoMapError {
    #[error("parse error: {0}")]
    Parse(#[from] ParseError),

    #[error("cache error: {0}")]
    Cache(#[from] CacheError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("repo map not built yet")]
    NotBuilt,
}

/// Builder for constructing and managing repository maps.
///
/// Provides a high-level API for building, caching, and querying
/// repository maps with file watching support for incremental updates.
pub struct RepoMapBuilder {
    /// Repository root path
    root: PathBuf,
    /// Cached repo map
    repo_map: Option<RepoMap>,
    /// Cache manager
    cache: RepoMapCache,
    /// Parser instance
    parser: TreeSitterParser,
    /// PageRank scorer
    scorer: PageRankScorer,
    /// Whether to use cache
    use_cache: bool,
}

impl RepoMapBuilder {
    /// Create a new builder for a repository.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, RepoMapError> {
        let root = root.into();
        let parser = TreeSitterParser::new()?;

        Ok(Self {
            cache: RepoMapCache::new(&root),
            root,
            repo_map: None,
            parser,
            scorer: PageRankScorer::new(),
            use_cache: true,
        })
    }

    /// Disable caching.
    pub fn without_cache(mut self) -> Self {
        self.use_cache = false;
        self
    }

    /// Build or update the repo map.
    ///
    /// If caching is enabled, this will:
    /// 1. Try to load from cache
    /// 2. Check for pending file changes
    /// 3. Incrementally update changed files
    /// 4. Save back to cache
    ///
    /// If caching is disabled or cache is incompatible, performs a full build.
    pub fn build_or_update(&mut self) -> Result<&RepoMap, RepoMapError> {
        // Try to load from cache
        if self.use_cache {
            match self.cache.load() {
                Ok(Some(mut map)) => {
                    info!("Loaded repo map from cache");

                    // Check for pending updates from file watcher
                    let pending = self.cache.pending_updates();
                    if pending.has_updates() {
                        self.apply_updates(&mut map, pending)?;
                        self.cache.save(&map)?;
                    }

                    self.repo_map = Some(map);
                    return Ok(self.repo_map.as_ref().unwrap());
                }
                Ok(None) => {
                    debug!("No cache found, performing full build");
                }
                Err(CacheError::VersionMismatch { .. }) => {
                    info!("Cache version mismatch, performing full rebuild");
                }
                Err(e) => {
                    warn!("Cache load error: {}, performing full build", e);
                }
            }
        }

        // Full build
        self.rebuild()
    }

    /// Force a complete rebuild of the repo map.
    pub fn rebuild(&mut self) -> Result<&RepoMap, RepoMapError> {
        info!("Building repo map for {:?}", self.root);

        let mut map = RepoMap::new(&self.root);

        // Walk the repository
        let walker = WalkBuilder::new(&self.root)
            .git_ignore(true)
            .hidden(false)
            .build();

        let mut file_count = 0;
        let mut parse_errors = 0;

        for entry in walker.flatten() {
            let path = entry.path();

            // Skip directories
            if !path.is_file() {
                continue;
            }

            // Check if it's a supported language
            let language = Language::from_path(path);
            if !language.is_supported() {
                continue;
            }

            // Parse the file
            match self.parse_file(path) {
                Ok(file_info) => {
                    map.add_file(file_info);
                    file_count += 1;
                }
                Err(e) => {
                    debug!("Failed to parse {:?}: {}", path, e);
                    parse_errors += 1;
                }
            }
        }

        info!(
            "Parsed {} files ({} errors)",
            file_count, parse_errors
        );

        // Build dependency graph and compute ranks
        let graph = DependencyGraph::build(&map);
        let ranks = self.scorer.compute(&graph);
        map.ranks = ranks;

        info!(
            "Built dependency graph: {} files, {} edges",
            graph.file_count(),
            graph.edge_count()
        );

        // Save to cache
        if self.use_cache {
            if let Err(e) = self.cache.save(&map) {
                warn!("Failed to save cache: {}", e);
            }
        }

        self.repo_map = Some(map);
        Ok(self.repo_map.as_ref().unwrap())
    }

    /// Start watching the repository for file changes.
    pub fn start_watching(&mut self) -> Result<(), RepoMapError> {
        self.cache.start_watching()?;
        Ok(())
    }

    /// Stop watching the repository.
    pub fn stop_watching(&mut self) {
        self.cache.stop_watching();
    }

    /// Check if there are pending updates.
    pub fn has_pending_updates(&self) -> bool {
        self.cache.has_pending_updates()
    }

    /// Get the current repo map (if built).
    pub fn repo_map(&self) -> Option<&RepoMap> {
        self.repo_map.as_ref()
    }

    /// Serialize the repo map for prompt injection.
    ///
    /// Returns a compact text representation of the most important files
    /// and their symbols, limited by token budget.
    pub fn serialize_for_prompt(&self, token_budget: usize) -> Result<String, RepoMapError> {
        let map = self.repo_map.as_ref().ok_or(RepoMapError::NotBuilt)?;
        let config = SerializeConfig::with_budget(token_budget);
        Ok(RepoMapSerializer::serialize_for_prompt(map, &config))
    }

    /// Serialize with query boosting for specific files.
    pub fn serialize_with_focus(
        &self,
        focus_files: &[PathBuf],
        query: Option<&str>,
        config: &SerializeConfig,
    ) -> Result<String, RepoMapError> {
        let map = self.repo_map.as_ref().ok_or(RepoMapError::NotBuilt)?;

        // Recompute ranks with focus boosting if needed
        if !focus_files.is_empty() {
            let graph = DependencyGraph::build(map);
            let _boosted_ranks = self.scorer.compute_with_query(&graph, focus_files);
            // Note: we don't modify the stored map, just use for this serialization
        }

        Ok(RepoMapSerializer::serialize_for_tool(
            map,
            Some(focus_files),
            query,
            config,
        ))
    }

    /// Parse a single file and return its info.
    fn parse_file(&mut self, path: &Path) -> Result<FileInfo, RepoMapError> {
        let content = std::fs::read_to_string(path)?;
        let relative = self.make_relative(path);

        let mut file_info = self.parser.parse_file(&relative, &content)?;

        // Add file metadata
        if let Ok(metadata) = std::fs::metadata(path) {
            file_info = file_info
                .with_mtime(metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH))
                .with_size(metadata.len());
        }

        // Update path to be relative
        file_info.path = relative;

        Ok(file_info)
    }

    /// Apply pending updates to the repo map.
    fn apply_updates(
        &mut self,
        map: &mut RepoMap,
        pending: PendingUpdates,
    ) -> Result<(), RepoMapError> {
        info!(
            "Applying {} modifications, {} deletions",
            pending.modified.len(),
            pending.deleted.len()
        );

        // Remove deleted files
        for path in pending.deleted {
            map.files.remove(&path);
        }

        // Re-parse modified files
        for relative_path in pending.modified {
            let full_path = self.root.join(&relative_path);
            match self.parse_file(&full_path) {
                Ok(file_info) => {
                    map.files.insert(relative_path, file_info);
                }
                Err(e) => {
                    debug!("Failed to parse modified file {:?}: {}", relative_path, e);
                    map.files.remove(&relative_path);
                }
            }
        }

        // Recompute ranks
        let graph = DependencyGraph::build(map);
        map.ranks = self.scorer.compute(&graph);

        Ok(())
    }

    /// Make a path relative to the repository root.
    fn make_relative(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.root)
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| path.to_path_buf())
    }

    /// Clear the cache file.
    pub fn clear_cache(&self) -> Result<(), RepoMapError> {
        self.cache.clear()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_project() -> TempDir {
        let temp = TempDir::new().unwrap();

        // Create a simple Rust project structure
        let src = temp.path().join("src");
        fs::create_dir_all(&src).unwrap();

        fs::write(
            src.join("main.rs"),
            r#"
use crate::lib::hello;

fn main() {
    hello();
}
"#,
        )
        .unwrap();

        fs::write(
            src.join("lib.rs"),
            r#"
pub fn hello() {
    println!("Hello!");
}

pub struct Config {
    pub name: String,
}
"#,
        )
        .unwrap();

        temp
    }

    #[test]
    fn test_builder_new() {
        let temp = create_test_project();
        let builder = RepoMapBuilder::new(temp.path()).unwrap();
        assert!(builder.repo_map().is_none());
    }

    #[test]
    fn test_builder_build() {
        let temp = create_test_project();
        let mut builder = RepoMapBuilder::new(temp.path()).unwrap().without_cache();

        let map = builder.rebuild().unwrap();

        assert!(map.file_count() >= 2);
        assert!(map.symbol_count() > 0);
    }

    #[test]
    fn test_builder_serialize() {
        let temp = create_test_project();
        let mut builder = RepoMapBuilder::new(temp.path()).unwrap().without_cache();

        builder.rebuild().unwrap();
        let output = builder.serialize_for_prompt(2000).unwrap();

        assert!(!output.is_empty());
        assert!(output.contains("main") || output.contains("lib"));
    }

    #[test]
    fn test_builder_caching() {
        let temp = create_test_project();

        // First build
        {
            let mut builder = RepoMapBuilder::new(temp.path()).unwrap();
            builder.rebuild().unwrap();
        }

        // Second build should load from cache
        {
            let mut builder = RepoMapBuilder::new(temp.path()).unwrap();
            let map = builder.build_or_update().unwrap();
            assert!(map.file_count() >= 2);
        }

        // Clear cache
        {
            let builder = RepoMapBuilder::new(temp.path()).unwrap();
            builder.clear_cache().unwrap();
        }
    }

    #[test]
    fn test_language_detection() {
        assert_eq!(Language::from_extension("rs"), Language::Rust);
        assert_eq!(Language::from_extension("ts"), Language::TypeScript);
        assert_eq!(Language::from_extension("py"), Language::Python);
        assert_eq!(Language::from_extension("unknown"), Language::Unknown);
    }
}
