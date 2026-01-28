//! Shell execution tool.
//!
//! Provides a sandboxed shell execution tool with timeout support.

use std::process::Stdio;
use std::time::Duration;

use agentik_core::tool::ToolCategory;
use agentik_core::{ToolCall, ToolDefinition, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

use crate::registry::{Tool, ToolContext};
use crate::ToolError;

/// Default timeout in seconds (2 minutes).
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Maximum timeout in seconds (10 minutes).
const MAX_TIMEOUT_SECS: u64 = 600;

/// Maximum output size in bytes (30KB).
const MAX_OUTPUT_SIZE: usize = 30_000;

/// Tool for executing shell commands.
///
/// Executes commands in a bash shell with configurable timeout.
/// Includes sandbox validation to block dangerous commands.
pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "Bash",
            "Executes a bash command with optional timeout. Commands are validated \
             against a blocklist for safety. Output is captured and returned.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (max 600000ms / 10 minutes). Defaults to 120000ms (2 minutes)."
                },
                "working_dir": {
                    "type": "string",
                    "description": "Working directory for the command. Defaults to current directory."
                },
                "description": {
                    "type": "string",
                    "description": "Optional description of what the command does"
                }
            },
            "required": ["command"]
        }))
        .with_category(ToolCategory::Shell)
        .requires_approval()
        .destructive()
    }

    fn validate(&self, arguments: &serde_json::Value) -> Result<(), ToolError> {
        if arguments.get("command").and_then(|v| v.as_str()).is_none() {
            return Err(ToolError::missing_param("command"));
        }
        Ok(())
    }

    async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // Check if shell is allowed
        if !ctx.sandbox.allow_shell {
            return Err(ToolError::ShellNotAllowed);
        }

        let command = call.arguments["command"]
            .as_str()
            .ok_or_else(|| ToolError::missing_param("command"))?;

        // Check for blocked commands
        if ctx.sandbox.is_command_blocked(command) {
            return Err(ToolError::BlockedCommand(command.to_string()));
        }

        // Parse timeout
        let timeout_ms = call
            .arguments
            .get("timeout")
            .and_then(|v| v.as_u64())
            .map(|ms| ms.min(MAX_TIMEOUT_SECS * 1000))
            .unwrap_or(DEFAULT_TIMEOUT_SECS * 1000);

        let timeout_secs = timeout_ms / 1000;

        // Determine working directory
        let working_dir = call
            .arguments
            .get("working_dir")
            .and_then(|v| v.as_str())
            .map(|p| ctx.resolve_path(p))
            .unwrap_or_else(|| ctx.working_dir.clone());

        // Execute the command
        let result = execute_command(command, &working_dir, timeout_secs).await;

        match result {
            Ok(output) => {
                let truncated = if output.len() > MAX_OUTPUT_SIZE {
                    format!(
                        "{}...\n\n[Output truncated at {} characters]",
                        &output[..MAX_OUTPUT_SIZE],
                        MAX_OUTPUT_SIZE
                    )
                } else {
                    output
                };
                Ok(ToolResult::success(&call.id, truncated))
            }
            Err(e) => {
                // Return error as a failed tool result rather than propagating
                Ok(ToolResult::error(&call.id, e.to_string()))
            }
        }
    }
}

/// Execute a command with timeout.
async fn execute_command(
    command: &str,
    working_dir: &std::path::Path,
    timeout_secs: u64,
) -> Result<String, ToolError> {
    let mut child = Command::new("bash")
        .arg("-c")
        .arg(command)
        .current_dir(working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let timeout_duration = Duration::from_secs(timeout_secs);

    // Wait for completion with timeout
    let status = match timeout(timeout_duration, child.wait()).await {
        Ok(result) => result?,
        Err(_) => {
            // Kill the process on timeout
            let _ = child.kill().await;
            return Err(ToolError::Timeout(timeout_secs));
        }
    };

    // Read stdout and stderr
    let mut stdout_output = String::new();
    let mut stderr_output = String::new();

    if let Some(mut stdout) = child.stdout {
        stdout.read_to_string(&mut stdout_output).await?;
    }

    if let Some(mut stderr) = child.stderr {
        stderr.read_to_string(&mut stderr_output).await?;
    }

    // Combine output
    let mut output = String::new();

    if !stdout_output.is_empty() {
        output.push_str(&stdout_output);
    }

    if !stderr_output.is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str("[stderr]\n");
        output.push_str(&stderr_output);
    }

    // Add exit code if non-zero
    if !status.success() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&format!("[exit code: {}]", status.code().unwrap_or(-1)));
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_context() -> (TempDir, ToolContext) {
        let dir = TempDir::new().unwrap();
        let ctx = ToolContext::new(dir.path());
        (dir, ctx)
    }

    #[tokio::test]
    async fn test_bash_simple_command() {
        let (_dir, ctx) = setup_context();
        let tool = BashTool;

        let call = ToolCall::new("test", "Bash", json!({ "command": "echo 'hello world'" }));

        let result = tool.execute(&call, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("hello world"));
    }

    #[tokio::test]
    async fn test_bash_command_with_exit_code() {
        let (_dir, ctx) = setup_context();
        let tool = BashTool;

        let call = ToolCall::new("test", "Bash", json!({ "command": "exit 1" }));

        let result = tool.execute(&call, &ctx).await.unwrap();
        // Command failed but tool execution succeeded
        assert!(!result.success || result.output.contains("exit code: 1"));
    }

    #[tokio::test]
    async fn test_bash_command_with_stderr() {
        let (_dir, ctx) = setup_context();
        let tool = BashTool;

        let call = ToolCall::new(
            "test",
            "Bash",
            json!({ "command": "echo 'error message' >&2" }),
        );

        let result = tool.execute(&call, &ctx).await.unwrap();
        assert!(result.output.contains("error message"));
        assert!(result.output.contains("[stderr]"));
    }

    #[tokio::test]
    async fn test_bash_blocked_command() {
        let (_dir, ctx) = setup_context();
        let tool = BashTool;

        let call = ToolCall::new("test", "Bash", json!({ "command": "rm -rf /" }));

        let result = tool.execute(&call, &ctx).await;
        assert!(matches!(result, Err(ToolError::BlockedCommand(_))));
    }

    #[tokio::test]
    async fn test_bash_timeout() {
        let (_dir, ctx) = setup_context();
        let tool = BashTool;

        // Very short timeout
        let call = ToolCall::new(
            "test",
            "Bash",
            json!({
                "command": "sleep 10",
                "timeout": 100  // 100ms timeout
            }),
        );

        let result = tool.execute(&call, &ctx).await.unwrap();
        // Should fail due to timeout
        assert!(result.error.is_some() || result.output.contains("timed out"));
    }

    #[tokio::test]
    async fn test_bash_working_dir() {
        let (dir, ctx) = setup_context();
        let tool = BashTool;

        // Create a subdirectory
        let subdir = dir.path().join("subdir");
        std::fs::create_dir(&subdir).unwrap();
        std::fs::write(subdir.join("test.txt"), "content").unwrap();

        let call = ToolCall::new(
            "test",
            "Bash",
            json!({
                "command": "ls",
                "working_dir": subdir.to_string_lossy()
            }),
        );

        let result = tool.execute(&call, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("test.txt"));
    }

    #[tokio::test]
    async fn test_bash_shell_not_allowed() {
        let (_dir, mut ctx) = setup_context();
        ctx.sandbox.allow_shell = false;
        let tool = BashTool;

        let call = ToolCall::new("test", "Bash", json!({ "command": "echo hello" }));

        let result = tool.execute(&call, &ctx).await;
        assert!(matches!(result, Err(ToolError::ShellNotAllowed)));
    }

    #[tokio::test]
    async fn test_bash_piped_command() {
        let (_dir, ctx) = setup_context();
        let tool = BashTool;

        let call = ToolCall::new(
            "test",
            "Bash",
            json!({ "command": "echo 'line1\nline2\nline3' | wc -l" }),
        );

        let result = tool.execute(&call, &ctx).await.unwrap();
        assert!(result.success);
        // Should contain "3" (three lines)
        assert!(result.output.trim().contains("3"));
    }
}
