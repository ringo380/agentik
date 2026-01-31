//! MCP transport implementations.
//!
//! This module provides the transport layer for communicating with MCP servers.
//! The primary transport is stdio, which spawns a child process and communicates
//! via stdin/stdout using newline-delimited JSON.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tracing::{debug, warn};

use crate::error::TransportError;

/// Trait for MCP transport implementations.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Send a message to the server.
    async fn send(&mut self, message: &str) -> Result<(), TransportError>;

    /// Receive a message from the server.
    async fn receive(&mut self) -> Result<String, TransportError>;

    /// Close the transport connection.
    async fn close(&mut self) -> Result<(), TransportError>;

    /// Check if the transport is connected.
    fn is_connected(&self) -> bool;
}

/// Standard I/O transport for MCP servers.
///
/// This transport spawns a child process and communicates via stdin/stdout
/// using newline-delimited JSON messages.
pub struct StdioTransport {
    /// The child process.
    child: Child,
    /// Stdin writer for sending messages.
    stdin: ChildStdin,
    /// Buffered stdout reader for receiving messages.
    stdout: BufReader<ChildStdout>,
    /// Whether the transport is connected.
    connected: bool,
}

impl StdioTransport {
    /// Spawn a new stdio transport.
    ///
    /// # Arguments
    ///
    /// * `command` - The command to execute (e.g., "npx" or "/path/to/server")
    /// * `args` - Command arguments
    /// * `env` - Environment variables to set for the child process
    /// * `working_dir` - Optional working directory for the child process
    pub async fn spawn(
        command: &str,
        args: &[String],
        env: HashMap<String, String>,
        working_dir: Option<&PathBuf>,
    ) -> Result<Self, TransportError> {
        debug!(
            command = command,
            args = ?args,
            "Spawning MCP server process"
        );

        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit()) // Let stderr pass through for debugging
            .kill_on_drop(true); // Ensure child is killed when transport is dropped

        // Set environment variables
        for (key, value) in &env {
            cmd.env(key, value);
        }

        // Set working directory if specified
        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        }

        let mut child = cmd.spawn().map_err(TransportError::SpawnFailed)?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| TransportError::SpawnFailed(std::io::Error::other(
                "Failed to capture stdin",
            )))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| TransportError::SpawnFailed(std::io::Error::other(
                "Failed to capture stdout",
            )))?;

        let stdout = BufReader::new(stdout);

        debug!("MCP server process spawned successfully");

        Ok(Self {
            child,
            stdin,
            stdout,
            connected: true,
        })
    }

    /// Get the process ID of the child process.
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn send(&mut self, message: &str) -> Result<(), TransportError> {
        if !self.connected {
            return Err(TransportError::NotConnected);
        }

        debug!(message = message, "Sending message to MCP server");

        // Write the message followed by a newline
        self.stdin
            .write_all(message.as_bytes())
            .await
            .map_err(TransportError::WriteError)?;

        self.stdin
            .write_all(b"\n")
            .await
            .map_err(TransportError::WriteError)?;

        // Flush to ensure the message is sent immediately
        self.stdin
            .flush()
            .await
            .map_err(TransportError::WriteError)?;

        Ok(())
    }

    async fn receive(&mut self) -> Result<String, TransportError> {
        if !self.connected {
            return Err(TransportError::NotConnected);
        }

        let mut line = String::new();
        let bytes_read = self
            .stdout
            .read_line(&mut line)
            .await
            .map_err(TransportError::ReadError)?;

        if bytes_read == 0 {
            self.connected = false;
            return Err(TransportError::ConnectionClosed);
        }

        // Remove the trailing newline
        let message = line.trim_end().to_string();

        debug!(message = message, "Received message from MCP server");

        Ok(message)
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        if !self.connected {
            return Ok(());
        }

        debug!("Closing MCP server transport");
        self.connected = false;

        // Try to terminate gracefully first
        if let Some(pid) = self.child.id() {
            debug!(pid = pid, "Sending SIGTERM to MCP server");

            // On Unix, send SIGTERM. On Windows, just kill.
            #[cfg(unix)]
            {
                use nix::sys::signal::{kill, Signal};
                use nix::unistd::Pid;

                let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);

                // Wait a short time for graceful shutdown
                tokio::select! {
                    _ = self.child.wait() => {
                        debug!("MCP server exited gracefully");
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(2)) => {
                        warn!("MCP server did not exit gracefully, killing");
                        let _ = self.child.kill().await;
                    }
                }
            }

            #[cfg(not(unix))]
            {
                // On non-Unix platforms, just kill the process
                let _ = self.child.kill().await;
            }
        }

        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        if self.connected {
            // Mark as disconnected to prevent further use
            self.connected = false;
            // The kill_on_drop(true) setting will handle killing the child
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stdio_transport_echo() {
        // Use 'cat' as a simple echo server for testing
        let transport = StdioTransport::spawn(
            "cat",
            &[],
            HashMap::new(),
            None,
        )
        .await;

        // This may fail on Windows if 'cat' is not available
        if let Ok(mut transport) = transport {
            assert!(transport.is_connected());

            // Send a message
            transport.send(r#"{"test": "hello"}"#).await.unwrap();

            // Receive the echoed message
            let response = transport.receive().await.unwrap();
            assert_eq!(response, r#"{"test": "hello"}"#);

            // Close the transport
            transport.close().await.unwrap();
            assert!(!transport.is_connected());
        }
    }

    #[tokio::test]
    async fn test_transport_not_connected() {
        let transport = StdioTransport::spawn(
            "cat",
            &[],
            HashMap::new(),
            None,
        )
        .await;

        if let Ok(mut transport) = transport {
            transport.close().await.unwrap();

            // Should error when not connected
            let result = transport.send("test").await;
            assert!(matches!(result, Err(TransportError::NotConnected)));

            let result = transport.receive().await;
            assert!(matches!(result, Err(TransportError::NotConnected)));
        }
    }
}
