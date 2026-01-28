//! # agentik-tools
//!
//! Built-in tool implementations for Agentik.
//!
//! This crate provides:
//! - File operations (read, write, edit, glob, grep)
//! - Shell execution (sandboxed)
//! - Git operations
//! - Web fetch and search
//!
//! ## Architecture
//!
//! Tools implement the [`Tool`] trait and are registered with a [`ToolRegistry`].
//! The registry manages tool lookup by name and category, and provides execution
//! with timing and validation.
//!
//! ## Example
//!
//! ```ignore
//! use agentik_tools::{ToolRegistry, ToolContext, file_ops::ReadTool};
//! use std::sync::Arc;
//!
//! let mut registry = ToolRegistry::new();
//! registry.register(Arc::new(ReadTool));
//!
//! let ctx = ToolContext::new("/path/to/project");
//! let call = ToolCall::new("call_1", "Read", json!({"file_path": "src/main.rs"}));
//! let result = registry.execute(&call, &ctx).await?;
//! ```

use std::path::PathBuf;
use thiserror::Error;

pub mod external;
pub mod file_ops;
pub mod git;
pub mod registry;
pub mod shell;
pub mod web;

pub use registry::{SandboxConfig, Tool, ToolContext, ToolRegistry};

// Re-export tools for convenience
pub use file_ops::{EditTool, GlobTool, GrepTool, ReadTool, WriteTool};
pub use git::{GitAddTool, GitCommitTool, GitDiffTool, GitLogTool, GitStatusTool};
pub use shell::BashTool;

use std::sync::Arc;

impl ToolRegistry {
    /// Create a registry with all built-in tools registered.
    pub fn with_builtins() -> Self {
        let mut registry = Self::new();

        // File operations
        registry.register(Arc::new(ReadTool));
        registry.register(Arc::new(WriteTool));
        registry.register(Arc::new(EditTool));
        registry.register(Arc::new(GlobTool));
        registry.register(Arc::new(GrepTool));

        // Shell
        registry.register(Arc::new(BashTool));

        // Git
        registry.register(Arc::new(GitStatusTool));
        registry.register(Arc::new(GitDiffTool));
        registry.register(Arc::new(GitLogTool));
        registry.register(Arc::new(GitAddTool));
        registry.register(Arc::new(GitCommitTool));

        registry
    }
}

/// Errors that can occur during tool execution.
#[derive(Debug, Error)]
pub enum ToolError {
    /// Tool was not found in the registry.
    #[error("tool not found: {0}")]
    NotFound(String),

    /// Invalid arguments provided to the tool.
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),

    /// Required parameter is missing.
    #[error("missing required parameter: {0}")]
    MissingParameter(String),

    /// Parameter has wrong type.
    #[error("parameter '{0}' has wrong type: expected {1}")]
    WrongType(String, String),

    /// File system I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Path is outside the allowed sandbox.
    #[error("path outside sandbox: {0}")]
    SandboxViolation(PathBuf),

    /// Command is blocked by sandbox configuration.
    #[error("command blocked: {0}")]
    BlockedCommand(String),

    /// Shell execution is not allowed.
    #[error("shell execution not allowed")]
    ShellNotAllowed,

    /// Network access is not allowed.
    #[error("network access not allowed")]
    NetworkNotAllowed,

    /// Operation timed out.
    #[error("operation timed out after {0} seconds")]
    Timeout(u64),

    /// Operation requires user approval.
    #[error("operation requires approval: {0}")]
    RequiresApproval(String),

    /// Git operation error.
    #[error("git error: {0}")]
    Git(#[from] git2::Error),

    /// Regex pattern error.
    #[error("invalid regex pattern: {0}")]
    Regex(#[from] regex::Error),

    /// Glob pattern error.
    #[error("invalid glob pattern: {0}")]
    Glob(#[from] globset::Error),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// The target string for edit was not found.
    #[error("string not found in file: {0}")]
    StringNotFound(String),

    /// The target string for edit was found multiple times.
    #[error("string found multiple times ({0} occurrences) - must be unique")]
    MultipleMatches(usize),

    /// Generic execution error.
    #[error("execution error: {0}")]
    Execution(String),
}

impl ToolError {
    /// Create an invalid arguments error.
    pub fn invalid_args(msg: impl Into<String>) -> Self {
        Self::InvalidArguments(msg.into())
    }

    /// Create a missing parameter error.
    pub fn missing_param(name: impl Into<String>) -> Self {
        Self::MissingParameter(name.into())
    }

    /// Create a wrong type error.
    pub fn wrong_type(param: impl Into<String>, expected: impl Into<String>) -> Self {
        Self::WrongType(param.into(), expected.into())
    }

    /// Create an execution error.
    pub fn execution(msg: impl Into<String>) -> Self {
        Self::Execution(msg.into())
    }
}
