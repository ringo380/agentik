//! Repository map tool for codebase context.
//!
//! Provides the GetRepoMap tool for querying the repository structure
//! and getting context about files and their relationships.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use agentik_core::tool::ToolCategory;
use agentik_core::{ToolCall, ToolDefinition, ToolResult};
use agentik_repomap::{
    DependencyGraph, PageRankScorer, RepoMap, RepoMapSerializer, SerializeConfig,
};
use async_trait::async_trait;
use serde_json::json;

use crate::registry::{Tool, ToolContext};
use crate::ToolError;

/// Tool for querying the repository map.
///
/// Returns information about the repository structure, including
/// file rankings and symbol information. Supports focus files for
/// query-boosted rankings.
pub struct GetRepoMapTool {
    /// Shared repo map (should be set by the agent)
    repo_map: Arc<RwLock<Option<RepoMap>>>,
}

impl GetRepoMapTool {
    /// Create a new GetRepoMap tool with a shared repo map.
    pub fn new(repo_map: Arc<RwLock<Option<RepoMap>>>) -> Self {
        Self { repo_map }
    }

    /// Create a tool with an empty repo map (for testing).
    pub fn empty() -> Self {
        Self {
            repo_map: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the repo map.
    pub fn set_repo_map(&self, map: RepoMap) {
        let mut guard = self.repo_map.write().unwrap();
        *guard = Some(map);
    }
}

#[async_trait]
impl Tool for GetRepoMapTool {
    fn name(&self) -> &str {
        "GetRepoMap"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "GetRepoMap",
            "Get repository map showing file structure, rankings, and symbols. \
             Use this to understand the codebase structure and find relevant files. \
             Optionally provide focus_files to boost their rankings and see related files.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "focus_files": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional list of file paths to focus on. These files and their neighbors will be boosted in the ranking."
                },
                "query": {
                    "type": "string",
                    "description": "Optional search query to filter files by path or symbol name."
                },
                "max_files": {
                    "type": "integer",
                    "description": "Maximum number of files to return. Default is 30."
                },
                "include_symbols": {
                    "type": "boolean",
                    "description": "Whether to include symbol information (functions, types). Default is true."
                }
            },
            "required": []
        }))
        .with_category(ToolCategory::FileSystem)
    }

    async fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // Get the repo map
        let guard = self.repo_map.read().unwrap();
        let map = guard
            .as_ref()
            .ok_or_else(|| ToolError::execution("Repository map not available. Build the repo map first."))?;

        // Parse parameters
        let focus_files: Vec<PathBuf> = call
            .arguments
            .get("focus_files")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default();

        let query = call
            .arguments
            .get("query")
            .and_then(|v| v.as_str());

        let max_files = call
            .arguments
            .get("max_files")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(30);

        let include_symbols = call
            .arguments
            .get("include_symbols")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // Build config
        let config = SerializeConfig::default()
            .max_files(max_files)
            .signatures(include_symbols);

        // Generate output
        let output = if !focus_files.is_empty() {
            // With focus files, recompute ranks with boosting
            let graph = DependencyGraph::build(map);
            let scorer = PageRankScorer::new();
            let boosted_ranks = scorer.compute_with_query(&graph, &focus_files);

            // Create a temporary map with boosted ranks
            let mut boosted_map = map.clone();
            boosted_map.ranks = boosted_ranks;

            RepoMapSerializer::serialize_for_tool(
                &boosted_map,
                Some(&focus_files),
                query,
                &config,
            )
        } else {
            RepoMapSerializer::serialize_for_tool(map, None, query, &config)
        };

        // Add summary
        let summary = format!(
            "\n---\nRepository: {} files, {} symbols\n",
            map.file_count(),
            map.symbol_count()
        );

        Ok(ToolResult::success(&call.id, format!("{}{}", output, summary)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentik_repomap::{FileInfo, Language, Symbol, SymbolKind};

    fn create_test_map() -> RepoMap {
        let mut map = RepoMap::new("/project");

        let mut file1 = FileInfo::new("src/main.rs", Language::Rust);
        file1.symbols = vec![
            Symbol::new("main", SymbolKind::Function, 1),
            Symbol::new("Config", SymbolKind::Struct, 10),
        ];

        let mut file2 = FileInfo::new("src/lib.rs", Language::Rust);
        file2.symbols = vec![
            Symbol::new("process", SymbolKind::Function, 1),
            Symbol::new("Handler", SymbolKind::Trait, 20),
        ];

        map.add_file(file1);
        map.add_file(file2);
        map.ranks.insert(PathBuf::from("src/main.rs"), 0.6);
        map.ranks.insert(PathBuf::from("src/lib.rs"), 0.4);

        map
    }

    #[tokio::test]
    async fn test_get_repo_map_basic() {
        let tool = GetRepoMapTool::empty();
        tool.set_repo_map(create_test_map());

        let call = ToolCall::new(
            "test_call",
            "GetRepoMap",
            json!({}),
        );

        let ctx = ToolContext::new("/project");
        let result = tool.execute(&call, &ctx).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("src/main.rs"));
        assert!(result.output.contains("src/lib.rs"));
    }

    #[tokio::test]
    async fn test_get_repo_map_with_query() {
        let tool = GetRepoMapTool::empty();
        tool.set_repo_map(create_test_map());

        let call = ToolCall::new(
            "test_call",
            "GetRepoMap",
            json!({"query": "Handler"}),
        );

        let ctx = ToolContext::new("/project");
        let result = tool.execute(&call, &ctx).await.unwrap();

        assert!(result.success);
        // Should include lib.rs which has Handler
        assert!(result.output.contains("lib.rs"));
    }

    #[tokio::test]
    async fn test_get_repo_map_with_focus() {
        let tool = GetRepoMapTool::empty();
        tool.set_repo_map(create_test_map());

        let call = ToolCall::new(
            "test_call",
            "GetRepoMap",
            json!({"focus_files": ["src/lib.rs"]}),
        );

        let ctx = ToolContext::new("/project");
        let result = tool.execute(&call, &ctx).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("Focus Files"));
        assert!(result.output.contains("lib.rs"));
    }

    #[tokio::test]
    async fn test_get_repo_map_max_files() {
        let tool = GetRepoMapTool::empty();
        tool.set_repo_map(create_test_map());

        let call = ToolCall::new(
            "test_call",
            "GetRepoMap",
            json!({"max_files": 1}),
        );

        let ctx = ToolContext::new("/project");
        let result = tool.execute(&call, &ctx).await.unwrap();

        assert!(result.success);
        // Output should be limited
    }

    #[tokio::test]
    async fn test_get_repo_map_not_built() {
        let tool = GetRepoMapTool::empty();

        let call = ToolCall::new(
            "test_call",
            "GetRepoMap",
            json!({}),
        );

        let ctx = ToolContext::new("/project");
        let result = tool.execute(&call, &ctx).await;

        assert!(result.is_err());
    }
}
