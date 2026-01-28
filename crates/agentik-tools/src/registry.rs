//! Tool registry for managing available tools.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agentik_core::tool::ToolCategory;
use agentik_core::{ToolCall, ToolDefinition, ToolResult};
use async_trait::async_trait;

use crate::ToolError;

/// Configuration for the execution sandbox.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Allowed directories for file operations
    pub allowed_paths: Vec<PathBuf>,
    /// Whether to allow network access
    pub allow_network: bool,
    /// Whether to allow shell execution
    pub allow_shell: bool,
    /// Commands that are blocked from execution
    pub blocked_commands: Vec<String>,
    /// Maximum execution time in seconds for shell commands
    pub max_execution_time: u64,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            allowed_paths: vec![],
            allow_network: true,
            allow_shell: true,
            blocked_commands: vec![
                "rm -rf /".to_string(),
                "rm -rf /*".to_string(),
                ":(){ :|:& };:".to_string(), // fork bomb
            ],
            max_execution_time: 120,
        }
    }
}

impl SandboxConfig {
    /// Create a sandbox config that allows access to a specific directory.
    pub fn for_directory(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        // Canonicalize to resolve symlinks (e.g., /var -> /private/var on macOS)
        let canonical = path.canonicalize().unwrap_or(path);
        Self {
            allowed_paths: vec![canonical],
            ..Default::default()
        }
    }

    /// Add an allowed path.
    pub fn with_allowed_path(mut self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        // Canonicalize to resolve symlinks
        let canonical = path.canonicalize().unwrap_or(path);
        self.allowed_paths.push(canonical);
        self
    }

    /// Set whether network access is allowed.
    pub fn with_network(mut self, allow: bool) -> Self {
        self.allow_network = allow;
        self
    }

    /// Set whether shell execution is allowed.
    pub fn with_shell(mut self, allow: bool) -> Self {
        self.allow_shell = allow;
        self
    }

    /// Add a blocked command.
    pub fn with_blocked_command(mut self, command: impl Into<String>) -> Self {
        self.blocked_commands.push(command.into());
        self
    }

    /// Set maximum execution time.
    pub fn with_max_execution_time(mut self, seconds: u64) -> Self {
        self.max_execution_time = seconds;
        self
    }

    /// Check if a path is allowed.
    pub fn is_path_allowed(&self, path: &std::path::Path) -> bool {
        if self.allowed_paths.is_empty() {
            return true; // No restrictions if no paths specified
        }

        // Try to canonicalize the path for comparison
        if let Ok(canonical) = path.canonicalize() {
            return self
                .allowed_paths
                .iter()
                .any(|allowed| canonical.starts_with(allowed));
        }

        // If we can't canonicalize (e.g., file/directory doesn't exist yet),
        // walk up the directory tree until we find an existing ancestor
        let mut current = path;
        while let Some(parent) = current.parent() {
            if let Ok(canonical_parent) = parent.canonicalize() {
                return self
                    .allowed_paths
                    .iter()
                    .any(|allowed| canonical_parent.starts_with(allowed));
            }
            current = parent;
        }

        false
    }

    /// Check if a command is blocked.
    pub fn is_command_blocked(&self, command: &str) -> bool {
        let normalized = command.trim().to_lowercase();
        self.blocked_commands
            .iter()
            .any(|blocked| normalized.contains(&blocked.to_lowercase()))
    }
}

/// Context for tool execution.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Working directory for relative paths
    pub working_dir: PathBuf,
    /// Sandbox configuration
    pub sandbox: SandboxConfig,
    /// Whether tools should require approval before destructive actions
    pub require_approval: bool,
}

impl Default for ToolContext {
    fn default() -> Self {
        Self {
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            sandbox: SandboxConfig::default(),
            require_approval: true,
        }
    }
}

impl ToolContext {
    /// Create a new context with a specific working directory.
    pub fn new(working_dir: impl Into<PathBuf>) -> Self {
        let working_dir = working_dir.into();
        Self {
            sandbox: SandboxConfig::for_directory(&working_dir),
            working_dir,
            require_approval: true,
        }
    }

    /// Resolve a path relative to the working directory.
    pub fn resolve_path(&self, path: &str) -> PathBuf {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            self.working_dir.join(path)
        }
    }

    /// Set the sandbox configuration.
    pub fn with_sandbox(mut self, sandbox: SandboxConfig) -> Self {
        self.sandbox = sandbox;
        self
    }

    /// Set whether approval is required.
    pub fn with_approval(mut self, require: bool) -> Self {
        self.require_approval = require;
        self
    }
}

/// Trait for implementing tools.
///
/// Tools are the primary way agents interact with the outside world.
/// Each tool has a name, definition (including JSON schema for parameters),
/// and an async execute method.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Get the unique name of this tool.
    fn name(&self) -> &str;

    /// Get the tool definition including parameter schema.
    fn definition(&self) -> ToolDefinition;

    /// Execute the tool with the given call and context.
    async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError>;

    /// Validate the arguments before execution.
    ///
    /// Default implementation does no validation.
    fn validate(&self, _arguments: &serde_json::Value) -> Result<(), ToolError> {
        Ok(())
    }
}

/// Registry of available tools.
///
/// Similar to ProviderRegistry, this manages available tools and provides
/// lookup by name and category.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    categories: HashMap<ToolCategory, Vec<String>>,
}

impl ToolRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            categories: HashMap::new(),
        }
    }

    /// Register a tool.
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        let category = tool.definition().category.clone();

        self.tools.insert(name.clone(), tool);

        self.categories.entry(category).or_default().push(name);
    }

    /// Get a tool by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Check if a tool exists.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// List all tool names.
    pub fn list(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Get all tool definitions.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    /// Get tools by category.
    pub fn by_category(&self, category: &ToolCategory) -> Vec<Arc<dyn Tool>> {
        self.categories
            .get(category)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|name| self.tools.get(name).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all registered categories.
    pub fn categories(&self) -> Vec<&ToolCategory> {
        self.categories.keys().collect()
    }

    /// Iterate over all tools.
    pub fn tools(&self) -> impl Iterator<Item = &Arc<dyn Tool>> {
        self.tools.values()
    }

    /// Get the number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Execute a tool call.
    pub async fn execute(
        &self,
        call: &ToolCall,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let tool = self
            .get(&call.name)
            .ok_or_else(|| ToolError::NotFound(call.name.clone()))?;

        // Validate arguments first
        tool.validate(&call.arguments)?;

        // Execute the tool
        let start = std::time::Instant::now();
        let mut result = tool.execute(call, ctx).await?;
        result.duration_ms = start.elapsed().as_millis() as u64;

        Ok(result)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentik_core::tool::ToolCategory;

    struct MockTool {
        name: String,
    }

    #[async_trait]
    impl Tool for MockTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn definition(&self) -> ToolDefinition {
            ToolDefinition::new(&self.name, "A mock tool for testing")
                .with_category(ToolCategory::External)
        }

        async fn execute(
            &self,
            call: &ToolCall,
            _ctx: &ToolContext,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success(
                &call.id,
                format!("Executed {}", self.name),
            ))
        }
    }

    #[test]
    fn test_registry_register_and_get() {
        let mut registry = ToolRegistry::new();
        let tool = Arc::new(MockTool {
            name: "test_tool".to_string(),
        });

        registry.register(tool);

        assert!(registry.contains("test_tool"));
        assert!(registry.get("test_tool").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_registry_list() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(MockTool {
            name: "tool_a".to_string(),
        }));
        registry.register(Arc::new(MockTool {
            name: "tool_b".to_string(),
        }));

        let names = registry.list();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"tool_a"));
        assert!(names.contains(&"tool_b"));
    }

    #[test]
    fn test_sandbox_path_check() {
        let sandbox = SandboxConfig::for_directory("/home/user/project");

        // These tests would need actual paths to work properly
        // For now, test the basic structure
        assert!(!sandbox.allowed_paths.is_empty());
    }

    #[test]
    fn test_sandbox_command_blocked() {
        let sandbox = SandboxConfig::default();

        assert!(sandbox.is_command_blocked("rm -rf /"));
        assert!(sandbox.is_command_blocked("RM -RF /"));
        assert!(!sandbox.is_command_blocked("ls -la"));
    }

    #[test]
    fn test_context_resolve_path() {
        let ctx = ToolContext::new("/home/user/project");

        // Relative path
        let resolved = ctx.resolve_path("src/main.rs");
        assert_eq!(resolved, PathBuf::from("/home/user/project/src/main.rs"));

        // Absolute path
        let resolved = ctx.resolve_path("/etc/passwd");
        assert_eq!(resolved, PathBuf::from("/etc/passwd"));
    }
}
