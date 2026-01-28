//! Tool execution engine with permission checking and approval workflows.
//!
//! This module provides [`ToolExecutor`], which bridges the agent loop with the
//! [`ToolRegistry`] by adding permission checking, user approval workflows, and
//! batch execution capabilities.
//!
//! ## Architecture
//!
//! ```text
//! Agent (orchestration)
//!   └─> ToolExecutor (permission + approval + execution)
//!       ├─> ToolRegistry (lookup and dispatch)
//!       │   └─> Tool implementations
//!       └─> PermissionHandler (approval callback)
//! ```
//!
//! ## Example
//!
//! ```ignore
//! use agentik_agent::{ToolExecutor, AutoApproveHandler};
//! use agentik_tools::{ToolRegistry, ToolContext};
//! use agentik_core::PermissionsConfig;
//! use std::sync::Arc;
//!
//! let registry = ToolRegistry::with_builtins();
//! let context = ToolContext::new("/path/to/project");
//! let handler = Arc::new(AutoApproveHandler);
//!
//! let executor = ToolExecutor::new(
//!     registry,
//!     context,
//!     PermissionsConfig::default(),
//!     AgentMode::Supervised,
//!     handler,
//! );
//!
//! let result = executor.execute(&tool_call).await;
//! ```

use std::sync::Arc;

use agentik_core::config::PermissionsConfig;
use agentik_core::{ToolCall, ToolDefinition, ToolResult};
use agentik_tools::{ToolContext, ToolRegistry};
use async_trait::async_trait;
use futures::future::join_all;
use tracing::{debug, info, warn};

use crate::modes::AgentMode;

/// Callback for handling permission requests during tool execution.
///
/// Implementations of this trait control how tool execution approval is handled.
/// The executor calls these methods at appropriate points in the execution flow.
#[async_trait]
pub trait PermissionHandler: Send + Sync {
    /// Request approval for a tool call.
    ///
    /// Returns `true` if the tool call is approved and should proceed,
    /// or `false` if it should be denied.
    ///
    /// # Arguments
    ///
    /// * `call` - The tool call requesting approval
    /// * `tool` - The definition of the tool being called
    async fn request_approval(&self, call: &ToolCall, tool: &ToolDefinition) -> bool;

    /// Notify that a tool is about to execute.
    ///
    /// Called after approval is granted, just before execution begins.
    /// Default implementation does nothing.
    fn on_execute(&self, _call: &ToolCall) {}

    /// Notify that a tool execution completed.
    ///
    /// Called after the tool finishes executing, regardless of success or failure.
    /// Default implementation does nothing.
    fn on_complete(&self, _call: &ToolCall, _result: &ToolResult) {}
}

/// A permission handler that automatically approves all tool calls.
///
/// Useful for autonomous mode or testing scenarios where no user
/// interaction is desired.
pub struct AutoApproveHandler;

#[async_trait]
impl PermissionHandler for AutoApproveHandler {
    async fn request_approval(&self, call: &ToolCall, _tool: &ToolDefinition) -> bool {
        debug!(tool = %call.name, "Auto-approving tool call");
        true
    }
}

/// A permission handler that denies all tool calls.
///
/// Useful for ask-only mode where no tool execution is allowed.
pub struct DenyAllHandler;

#[async_trait]
impl PermissionHandler for DenyAllHandler {
    async fn request_approval(&self, call: &ToolCall, _tool: &ToolDefinition) -> bool {
        debug!(tool = %call.name, "Denying tool call (DenyAllHandler)");
        false
    }
}

/// Reason why a tool execution was denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenialReason {
    /// Tool is in the always_deny list
    AlwaysDenied,
    /// Agent is in AskOnly mode
    AskOnlyMode,
    /// User declined approval
    UserDeclined,
    /// Tool not found in registry
    ToolNotFound,
}

impl std::fmt::Display for DenialReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DenialReason::AlwaysDenied => write!(f, "tool is in the always_deny list"),
            DenialReason::AskOnlyMode => write!(f, "agent is in ask-only mode"),
            DenialReason::UserDeclined => write!(f, "user declined approval"),
            DenialReason::ToolNotFound => write!(f, "tool not found"),
        }
    }
}

/// Tool executor with permission checking and approval workflows.
///
/// The `ToolExecutor` wraps a [`ToolRegistry`] and adds:
/// - Permission checking based on configuration and agent mode
/// - User approval workflows via [`PermissionHandler`]
/// - Lifecycle notifications (on_execute, on_complete)
/// - Batch execution with parallel processing
pub struct ToolExecutor {
    registry: ToolRegistry,
    context: ToolContext,
    permissions: PermissionsConfig,
    mode: AgentMode,
    handler: Arc<dyn PermissionHandler>,
}

impl ToolExecutor {
    /// Create a new tool executor.
    ///
    /// # Arguments
    ///
    /// * `registry` - Registry of available tools
    /// * `context` - Execution context (working directory, sandbox config)
    /// * `permissions` - Permission configuration (allow/deny lists)
    /// * `mode` - Agent operating mode
    /// * `handler` - Handler for permission requests
    pub fn new(
        registry: ToolRegistry,
        context: ToolContext,
        permissions: PermissionsConfig,
        mode: AgentMode,
        handler: Arc<dyn PermissionHandler>,
    ) -> Self {
        Self {
            registry,
            context,
            permissions,
            mode,
            handler,
        }
    }

    /// Get a reference to the underlying tool registry.
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// Get a mutable reference to the underlying tool registry.
    pub fn registry_mut(&mut self) -> &mut ToolRegistry {
        &mut self.registry
    }

    /// Get a reference to the tool context.
    pub fn context(&self) -> &ToolContext {
        &self.context
    }

    /// Get the current agent mode.
    pub fn mode(&self) -> AgentMode {
        self.mode
    }

    /// Set the agent mode.
    pub fn set_mode(&mut self, mode: AgentMode) {
        self.mode = mode;
    }

    /// Check if a tool is always denied.
    pub fn is_denied(&self, tool_name: &str) -> Option<DenialReason> {
        // Check if in always_deny list
        if self.permissions.always_deny.contains(&tool_name.to_string()) {
            return Some(DenialReason::AlwaysDenied);
        }

        // Check if in AskOnly mode (no tools allowed)
        if self.mode == AgentMode::AskOnly {
            return Some(DenialReason::AskOnlyMode);
        }

        None
    }

    /// Check if a tool requires approval before execution.
    ///
    /// Approval is required when ANY of these is true:
    /// 1. Tool definition has `requires_approval = true`
    /// 2. Tool name is in `permissions.require_confirm`
    /// 3. `AgentMode::Supervised` is active
    /// 4. Tool is destructive AND mode is not `Autonomous`
    pub fn requires_approval(&self, tool: &ToolDefinition) -> bool {
        // Tool explicitly requires approval
        if tool.requires_approval {
            return true;
        }

        // Tool is in require_confirm list
        if self
            .permissions
            .require_confirm
            .contains(&tool.name.to_string())
        {
            return true;
        }

        // Supervised mode requires approval for everything
        if self.mode == AgentMode::Supervised {
            return true;
        }

        // Destructive tools require approval unless in Autonomous mode
        if tool.is_destructive && self.mode != AgentMode::Autonomous {
            return true;
        }

        false
    }

    /// Check if a tool is auto-approved (no approval needed).
    fn is_auto_approved(&self, tool: &ToolDefinition) -> bool {
        // In Autonomous mode, non-destructive tools are auto-approved
        if self.mode == AgentMode::Autonomous && !tool.is_destructive {
            return true;
        }

        // Tools in default_allow list are auto-approved (unless they require approval)
        if self.permissions.default_allow.contains(&tool.name) && !self.requires_approval(tool) {
            return true;
        }

        false
    }

    /// Execute a single tool call with permission checking.
    ///
    /// This method:
    /// 1. Looks up the tool in the registry
    /// 2. Checks if the tool is denied
    /// 3. Checks if approval is required and requests it
    /// 4. Executes the tool if approved
    /// 5. Notifies the handler of completion
    ///
    /// # Returns
    ///
    /// A [`ToolResult`] indicating success or failure. If the tool is denied
    /// or approval is declined, an error result is returned.
    pub async fn execute(&self, call: &ToolCall) -> ToolResult {
        info!(tool = %call.name, call_id = %call.id, "Executing tool call");

        // Look up the tool
        let tool = match self.registry.get(&call.name) {
            Some(t) => t,
            None => {
                warn!(tool = %call.name, "Tool not found");
                return ToolResult::error(&call.id, format!("Tool not found: {}", call.name));
            }
        };

        let definition = tool.definition();

        // Check if tool is denied
        if let Some(reason) = self.is_denied(&call.name) {
            warn!(tool = %call.name, reason = %reason, "Tool execution denied");
            return ToolResult::error(&call.id, format!("Tool execution denied: {}", reason));
        }

        // Check if approval is required
        if self.requires_approval(&definition) && !self.is_auto_approved(&definition) {
            debug!(tool = %call.name, "Requesting approval for tool");
            let approved = self.handler.request_approval(call, &definition).await;

            if !approved {
                info!(tool = %call.name, "Tool approval declined");
                return ToolResult::error(
                    &call.id,
                    format!("Tool execution denied: {}", DenialReason::UserDeclined),
                );
            }
        }

        // Notify execution start
        self.handler.on_execute(call);

        // Execute the tool
        let result = match self.registry.execute(call, &self.context).await {
            Ok(result) => result,
            Err(e) => {
                warn!(tool = %call.name, error = %e, "Tool execution failed");
                ToolResult::error(&call.id, e.to_string())
            }
        };

        // Notify completion
        self.handler.on_complete(call, &result);

        info!(
            tool = %call.name,
            success = result.success,
            duration_ms = result.duration_ms,
            "Tool execution completed"
        );

        result
    }

    /// Execute multiple tool calls, potentially in parallel.
    ///
    /// This method analyzes the calls and executes them in parallel when safe.
    /// Currently, all calls are executed in parallel since tools are expected
    /// to be independent. Future versions may implement dependency detection.
    ///
    /// # Arguments
    ///
    /// * `calls` - Slice of tool calls to execute
    ///
    /// # Returns
    ///
    /// A vector of [`ToolResult`]s in the same order as the input calls.
    pub async fn execute_batch(&self, calls: &[ToolCall]) -> Vec<ToolResult> {
        if calls.is_empty() {
            return Vec::new();
        }

        if calls.len() == 1 {
            return vec![self.execute(&calls[0]).await];
        }

        info!(count = calls.len(), "Executing tool calls in batch");

        // Execute all calls in parallel
        let futures: Vec<_> = calls.iter().map(|call| self.execute(call)).collect();

        join_all(futures).await
    }
}

/// Builder for constructing a [`ToolExecutor`].
///
/// Provides a convenient API for configuring the executor with sensible defaults.
#[derive(Default)]
pub struct ExecutorBuilder {
    registry: Option<ToolRegistry>,
    working_dir: Option<std::path::PathBuf>,
    permissions: Option<PermissionsConfig>,
    mode: AgentMode,
}

impl ExecutorBuilder {
    /// Create a new executor builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Use a registry with all built-in tools registered.
    pub fn with_builtins(mut self) -> Self {
        self.registry = Some(ToolRegistry::with_builtins());
        self
    }

    /// Use a custom tool registry.
    pub fn with_registry(mut self, registry: ToolRegistry) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Set the working directory for tool execution.
    pub fn working_dir(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.working_dir = Some(path.into());
        self
    }

    /// Set the permissions configuration.
    pub fn permissions(mut self, config: PermissionsConfig) -> Self {
        self.permissions = Some(config);
        self
    }

    /// Set the agent mode.
    pub fn mode(mut self, mode: AgentMode) -> Self {
        self.mode = mode;
        self
    }

    /// Build the executor with the given permission handler.
    pub fn build(self, handler: Arc<dyn PermissionHandler>) -> ToolExecutor {
        let registry = self.registry.unwrap_or_default();
        let working_dir = self
            .working_dir
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
        let context = ToolContext::new(working_dir);
        let permissions = self.permissions.unwrap_or_default();

        ToolExecutor::new(registry, context, permissions, self.mode, handler)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentik_core::tool::ToolCategory;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::Mutex;

    /// A mock permission handler for testing.
    struct MockHandler {
        approve: AtomicBool,
        approval_count: AtomicUsize,
        execute_count: AtomicUsize,
        complete_count: AtomicUsize,
        last_tool: Mutex<Option<String>>,
    }

    impl MockHandler {
        fn new(approve: bool) -> Self {
            Self {
                approve: AtomicBool::new(approve),
                approval_count: AtomicUsize::new(0),
                execute_count: AtomicUsize::new(0),
                complete_count: AtomicUsize::new(0),
                last_tool: Mutex::new(None),
            }
        }

        fn approval_count(&self) -> usize {
            self.approval_count.load(Ordering::SeqCst)
        }

        fn execute_count(&self) -> usize {
            self.execute_count.load(Ordering::SeqCst)
        }

        fn complete_count(&self) -> usize {
            self.complete_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl PermissionHandler for MockHandler {
        async fn request_approval(&self, call: &ToolCall, _tool: &ToolDefinition) -> bool {
            self.approval_count.fetch_add(1, Ordering::SeqCst);
            *self.last_tool.lock().await = Some(call.name.clone());
            self.approve.load(Ordering::SeqCst)
        }

        fn on_execute(&self, _call: &ToolCall) {
            self.execute_count.fetch_add(1, Ordering::SeqCst);
        }

        fn on_complete(&self, _call: &ToolCall, _result: &ToolResult) {
            self.complete_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn create_test_tool_definition(name: &str, requires_approval: bool, destructive: bool) -> ToolDefinition {
        let mut def = ToolDefinition::new(name, "Test tool");
        def.category = ToolCategory::External;
        def.requires_approval = requires_approval;
        def.is_destructive = destructive;
        def
    }

    #[test]
    fn test_is_denied_always_deny() {
        let permissions = PermissionsConfig {
            always_deny: vec!["dangerous_tool".to_string()],
            ..Default::default()
        };

        let executor = ExecutorBuilder::new()
            .permissions(permissions)
            .build(Arc::new(AutoApproveHandler));

        assert_eq!(
            executor.is_denied("dangerous_tool"),
            Some(DenialReason::AlwaysDenied)
        );
        assert_eq!(executor.is_denied("safe_tool"), None);
    }

    #[test]
    fn test_is_denied_ask_only_mode() {
        let executor = ExecutorBuilder::new()
            .mode(AgentMode::AskOnly)
            .build(Arc::new(AutoApproveHandler));

        assert_eq!(
            executor.is_denied("any_tool"),
            Some(DenialReason::AskOnlyMode)
        );
    }

    #[test]
    fn test_requires_approval_explicit() {
        let executor = ExecutorBuilder::new()
            .mode(AgentMode::Autonomous)
            .build(Arc::new(AutoApproveHandler));

        let tool = create_test_tool_definition("test", true, false);
        assert!(executor.requires_approval(&tool));
    }

    #[test]
    fn test_requires_approval_require_confirm_list() {
        let permissions = PermissionsConfig {
            require_confirm: vec!["Write".to_string()],
            ..Default::default()
        };

        let executor = ExecutorBuilder::new()
            .permissions(permissions)
            .mode(AgentMode::Autonomous)
            .build(Arc::new(AutoApproveHandler));

        let tool = create_test_tool_definition("Write", false, false);
        assert!(executor.requires_approval(&tool));

        let other_tool = create_test_tool_definition("Read", false, false);
        assert!(!executor.requires_approval(&other_tool));
    }

    #[test]
    fn test_requires_approval_supervised_mode() {
        let executor = ExecutorBuilder::new()
            .mode(AgentMode::Supervised)
            .build(Arc::new(AutoApproveHandler));

        let tool = create_test_tool_definition("safe_tool", false, false);
        assert!(executor.requires_approval(&tool));
    }

    #[test]
    fn test_requires_approval_destructive_non_autonomous() {
        // In Planning mode (not Autonomous), destructive tools require approval
        let executor = ExecutorBuilder::new()
            .mode(AgentMode::Planning)
            .build(Arc::new(AutoApproveHandler));

        let destructive_tool = create_test_tool_definition("delete", false, true);
        assert!(executor.requires_approval(&destructive_tool));

        // In Autonomous mode, destructive tools don't require approval
        let autonomous_executor = ExecutorBuilder::new()
            .mode(AgentMode::Autonomous)
            .build(Arc::new(AutoApproveHandler));

        assert!(!autonomous_executor.requires_approval(&destructive_tool));
    }

    #[test]
    fn test_auto_approve_handler() {
        let handler = AutoApproveHandler;
        let call = ToolCall::new("1", "test", serde_json::json!({}));
        let tool = create_test_tool_definition("test", false, false);

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handler.request_approval(&call, &tool));
        assert!(result);
    }

    #[test]
    fn test_deny_all_handler() {
        let handler = DenyAllHandler;
        let call = ToolCall::new("1", "test", serde_json::json!({}));
        let tool = create_test_tool_definition("test", false, false);

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handler.request_approval(&call, &tool));
        assert!(!result);
    }

    #[tokio::test]
    async fn test_execute_tool_not_found() {
        let executor = ExecutorBuilder::new()
            .build(Arc::new(AutoApproveHandler));

        let call = ToolCall::new("1", "nonexistent", serde_json::json!({}));
        let result = executor.execute(&call).await;

        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn test_execute_denied_tool() {
        let permissions = PermissionsConfig {
            always_deny: vec!["Read".to_string()],
            ..Default::default()
        };

        let executor = ExecutorBuilder::new()
            .with_builtins()
            .permissions(permissions)
            .build(Arc::new(AutoApproveHandler));

        let call = ToolCall::new("1", "Read", serde_json::json!({"file_path": "/etc/passwd"}));
        let result = executor.execute(&call).await;

        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("denied"));
    }

    #[tokio::test]
    async fn test_execute_approval_declined() {
        let handler = Arc::new(MockHandler::new(false));

        let executor = ExecutorBuilder::new()
            .with_builtins()
            .mode(AgentMode::Supervised)
            .build(handler.clone());

        let call = ToolCall::new("1", "Read", serde_json::json!({"file_path": "test.txt"}));
        let result = executor.execute(&call).await;

        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("declined"));
        assert_eq!(handler.approval_count(), 1);
        // Should not have executed
        assert_eq!(handler.execute_count(), 0);
    }

    #[tokio::test]
    async fn test_execute_handler_callbacks() {
        let handler = Arc::new(MockHandler::new(true));

        let executor = ExecutorBuilder::new()
            .with_builtins()
            .mode(AgentMode::Supervised)
            .build(handler.clone());

        // Create a temp file to read
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("agentik_test_executor.txt");
        std::fs::write(&test_file, "test content").unwrap();

        let call = ToolCall::new(
            "1",
            "Read",
            serde_json::json!({"file_path": test_file.to_str().unwrap()}),
        );
        let result = executor.execute(&call).await;

        // Clean up
        let _ = std::fs::remove_file(&test_file);

        // In supervised mode, approval is always requested
        assert_eq!(handler.approval_count(), 1);
        // Should have executed and completed
        assert_eq!(handler.execute_count(), 1);
        assert_eq!(handler.complete_count(), 1);

        // Result depends on whether file read succeeded
        if result.success {
            assert!(result.output.contains("test content"));
        }
    }

    #[tokio::test]
    async fn test_execute_batch_empty() {
        let executor = ExecutorBuilder::new()
            .build(Arc::new(AutoApproveHandler));

        let results = executor.execute_batch(&[]).await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_execute_batch_multiple() {
        let executor = ExecutorBuilder::new()
            .build(Arc::new(AutoApproveHandler));

        let calls = vec![
            ToolCall::new("1", "nonexistent1", serde_json::json!({})),
            ToolCall::new("2", "nonexistent2", serde_json::json!({})),
            ToolCall::new("3", "nonexistent3", serde_json::json!({})),
        ];

        let results = executor.execute_batch(&calls).await;

        assert_eq!(results.len(), 3);
        // All should fail (tools don't exist)
        assert!(results.iter().all(|r| !r.success));
        // Results should match call IDs
        assert_eq!(results[0].tool_call_id, "1");
        assert_eq!(results[1].tool_call_id, "2");
        assert_eq!(results[2].tool_call_id, "3");
    }

    #[test]
    fn test_builder_defaults() {
        let executor = ExecutorBuilder::new()
            .build(Arc::new(AutoApproveHandler));

        assert!(executor.registry().is_empty());
        assert_eq!(executor.mode(), AgentMode::Autonomous);
    }

    #[test]
    fn test_builder_with_builtins() {
        let executor = ExecutorBuilder::new()
            .with_builtins()
            .build(Arc::new(AutoApproveHandler));

        assert!(!executor.registry().is_empty());
        assert!(executor.registry().contains("Read"));
        assert!(executor.registry().contains("Write"));
        assert!(executor.registry().contains("Bash"));
    }

    #[test]
    fn test_denial_reason_display() {
        assert_eq!(
            DenialReason::AlwaysDenied.to_string(),
            "tool is in the always_deny list"
        );
        assert_eq!(
            DenialReason::AskOnlyMode.to_string(),
            "agent is in ask-only mode"
        );
        assert_eq!(
            DenialReason::UserDeclined.to_string(),
            "user declined approval"
        );
        assert_eq!(DenialReason::ToolNotFound.to_string(), "tool not found");
    }
}
