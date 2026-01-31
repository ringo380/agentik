//! MCP-specific error types.

use thiserror::Error;

/// Errors that can occur during transport operations.
#[derive(Debug, Error)]
pub enum TransportError {
    /// Failed to spawn the child process.
    #[error("failed to spawn process: {0}")]
    SpawnFailed(std::io::Error),

    /// Failed to write to the transport.
    #[error("write error: {0}")]
    WriteError(std::io::Error),

    /// Failed to read from the transport.
    #[error("read error: {0}")]
    ReadError(std::io::Error),

    /// Connection was closed unexpectedly.
    #[error("connection closed")]
    ConnectionClosed,

    /// Transport is not connected.
    #[error("not connected")]
    NotConnected,

    /// Failed to terminate the process.
    #[error("failed to terminate process: {0}")]
    TerminateFailed(std::io::Error),
}

/// Errors that can occur during MCP operations.
#[derive(Debug, Error)]
pub enum McpError {
    /// Transport-level error.
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),

    /// Protocol-level error (malformed messages, etc.).
    #[error("protocol error: {0}")]
    Protocol(String),

    /// Server not found by name.
    #[error("server not found: {0}")]
    ServerNotFound(String),

    /// Tool not found on server.
    #[error("tool not found: {0}")]
    ToolNotFound(String),

    /// Server initialization failed.
    #[error("initialization failed: {0}")]
    InitializationFailed(String),

    /// Server returned an error response.
    #[error("server error (code {code}): {message}")]
    ServerError { code: i32, message: String },

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// I/O error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Connection is not in the correct state.
    #[error("invalid connection state: expected {expected}, got {actual}")]
    InvalidState { expected: String, actual: String },

    /// Request timed out.
    #[error("request timed out after {0} seconds")]
    Timeout(u64),

    /// Server already exists with this name.
    #[error("server already exists: {0}")]
    ServerAlreadyExists(String),
}

impl McpError {
    /// Create a protocol error.
    pub fn protocol(msg: impl Into<String>) -> Self {
        Self::Protocol(msg.into())
    }

    /// Create an initialization failed error.
    pub fn init_failed(msg: impl Into<String>) -> Self {
        Self::InitializationFailed(msg.into())
    }

    /// Create a server error from JSON-RPC error.
    pub fn server_error(code: i32, message: impl Into<String>) -> Self {
        Self::ServerError {
            code,
            message: message.into(),
        }
    }

    /// Create an invalid state error.
    pub fn invalid_state(expected: impl Into<String>, actual: impl Into<String>) -> Self {
        Self::InvalidState {
            expected: expected.into(),
            actual: actual.into(),
        }
    }
}

impl From<McpError> for agentik_core::Error {
    fn from(e: McpError) -> Self {
        agentik_core::Error::Mcp(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transport_error_display() {
        let err = TransportError::ConnectionClosed;
        assert_eq!(err.to_string(), "connection closed");
    }

    #[test]
    fn test_mcp_error_display() {
        let err = McpError::ServerNotFound("test-server".to_string());
        assert_eq!(err.to_string(), "server not found: test-server");

        let err = McpError::server_error(-32600, "Invalid request");
        assert_eq!(err.to_string(), "server error (code -32600): Invalid request");
    }

    #[test]
    fn test_mcp_error_to_core_error() {
        let err = McpError::protocol("test error");
        let core_err: agentik_core::Error = err.into();
        assert!(matches!(core_err, agentik_core::Error::Mcp(_)));
    }
}
