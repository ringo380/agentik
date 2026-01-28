//! Error types for Agentik.

use thiserror::Error;

/// Result type alias using AgentikError.
pub type Result<T> = std::result::Result<T, Error>;

/// Main error type for Agentik.
#[derive(Error, Debug)]
pub enum Error {
    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Provider error
    #[error("Provider error: {0}")]
    Provider(String),

    /// Session error
    #[error("Session error: {0}")]
    Session(String),

    /// Tool execution error
    #[error("Tool error: {0}")]
    Tool(String),

    /// MCP error
    #[error("MCP error: {0}")]
    Mcp(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Database error
    #[error("Database error: {0}")]
    Database(String),

    /// HTTP request error
    #[error("HTTP error: {0}")]
    Http(String),

    /// Validation error
    #[error("Validation error: {0}")]
    Validation(String),

    /// Not found error
    #[error("Not found: {0}")]
    NotFound(String),

    /// Permission denied
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}
