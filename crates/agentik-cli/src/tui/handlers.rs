//! Event and permission handlers for CLI integration.
//!
//! This module provides:
//! - [`CliEventHandler`]: Handles agent events (text streaming, tool status, usage display)
//! - [`CliPermissionHandler`]: Handles tool approval prompts (y/n/a/q)

use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::Mutex;
use std::time::Instant;

use agentik_agent::{AgentEventHandler, AgentResponse, PermissionHandler, TurnUsage};
use agentik_core::{ToolCall, ToolDefinition, ToolResult};
use async_trait::async_trait;

// ============================================================================
// CLI Event Handler
// ============================================================================

/// Event handler that streams output to the terminal.
///
/// Implements [`AgentEventHandler`] to provide real-time feedback during
/// agent execution. Text is streamed character by character, and tool
/// operations show status messages.
pub struct CliEventHandler {
    /// Track when tools start for duration calculation
    tool_start: Mutex<Option<Instant>>,
}

impl CliEventHandler {
    /// Create a new CLI event handler.
    pub fn new() -> Self {
        Self {
            tool_start: Mutex::new(None),
        }
    }
}

impl Default for CliEventHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentEventHandler for CliEventHandler {
    fn on_thinking(&self) {
        // Could show a spinner here, but we'll keep it simple
    }

    fn on_text_delta(&self, delta: &str) {
        print!("{}", delta);
        io::stdout().flush().ok();
    }

    fn on_tool_start(&self, call: &ToolCall) {
        // Store start time
        *self.tool_start.lock().unwrap() = Some(Instant::now());

        // Show tool starting message
        eprintln!("\n[Tool: {}] Starting...", call.name);
    }

    fn on_tool_complete(&self, call: &ToolCall, result: &ToolResult) {
        let duration = self
            .tool_start
            .lock()
            .unwrap()
            .take()
            .map(|start| start.elapsed().as_millis())
            .unwrap_or(result.duration_ms as u128);

        let status = if result.success { "OK" } else { "FAILED" };
        eprintln!("[Tool: {}] {} ({}ms)", call.name, status, duration);

        // Show error message if failed
        if !result.success {
            if let Some(ref error) = result.error {
                eprintln!("[Error: {}]", error);
            }
        }
    }

    fn on_usage(&self, usage: &TurnUsage) {
        eprintln!(
            "\n[Usage: {} in / {} out | ${:.4}]",
            usage.input_tokens, usage.output_tokens, usage.cost_usd
        );
    }

    fn on_compacting(&self) {
        eprintln!("[Compacting context...]");
    }

    fn on_error(&self, error: &agentik_agent::AgentError) {
        eprintln!("[Error: {}]", error);
    }

    fn on_complete(&self, _response: &AgentResponse) {
        // Print a newline after the response for better formatting
        println!();
    }
}

// ============================================================================
// CLI Permission Handler
// ============================================================================

/// Permission handler that prompts the user for approval.
///
/// Supports the following responses:
/// - `y` or `yes`: Approve this tool call
/// - `n` or `no`: Deny this tool call
/// - `a` or `always`: Approve and auto-approve future calls to this tool
/// - `q` or `quit`: Deny and exit the agent loop
///
/// Tools marked as "always approved" are tracked in memory for the session.
pub struct CliPermissionHandler {
    /// Tools that have been marked as always approved.
    always_approved: Mutex<HashSet<String>>,
}

impl CliPermissionHandler {
    /// Create a new CLI permission handler.
    pub fn new() -> Self {
        Self {
            always_approved: Mutex::new(HashSet::new()),
        }
    }

    /// Check if a tool is in the always-approved set.
    pub fn is_always_approved(&self, tool_name: &str) -> bool {
        self.always_approved.lock().unwrap().contains(tool_name)
    }

    /// Mark a tool as always approved.
    pub fn set_always_approved(&self, tool_name: &str) {
        self.always_approved
            .lock()
            .unwrap()
            .insert(tool_name.to_string());
    }

    /// Read user input for approval (sync blocking version).
    fn prompt_user_sync(
        tool_name: &str,
        tool_desc: &str,
        is_destructive: bool,
        args: &serde_json::Value,
    ) -> ApprovalResponse {
        // Show tool information
        eprintln!();
        eprintln!("╔══════════════════════════════════════════════════════════════╗");
        eprintln!("║  Tool Approval Required                                       ║");
        eprintln!("╠══════════════════════════════════════════════════════════════╣");
        eprintln!("║  Tool: {:<54} ║", tool_name);
        eprintln!("║  Description: {:<47} ║", truncate(tool_desc, 47));
        if is_destructive {
            eprintln!(
                "║  Warning: This tool is marked as DESTRUCTIVE                  ║"
            );
        }
        eprintln!("╠══════════════════════════════════════════════════════════════╣");
        eprintln!("║  Arguments:                                                   ║");

        // Pretty print arguments
        let args_str = serde_json::to_string_pretty(args).unwrap_or_default();
        for line in args_str.lines().take(10) {
            eprintln!("║    {:<58} ║", truncate(line, 58));
        }
        if args_str.lines().count() > 10 {
            eprintln!("║    ... (truncated)                                           ║");
        }

        eprintln!("╚══════════════════════════════════════════════════════════════╝");
        eprintln!();
        eprint!("Approve? [y]es / [n]o / [a]lways / [q]uit: ");
        io::stderr().flush().ok();

        // Read input
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return ApprovalResponse::Deny;
        }

        match input.trim().to_lowercase().as_str() {
            "y" | "yes" => ApprovalResponse::Approve,
            "n" | "no" | "" => ApprovalResponse::Deny,
            "a" | "always" => ApprovalResponse::AlwaysApprove,
            "q" | "quit" => ApprovalResponse::Quit,
            _ => {
                eprintln!("Unknown response, denying.");
                ApprovalResponse::Deny
            }
        }
    }
}

impl Default for CliPermissionHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Response from the user for approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalResponse {
    /// Approve this single call
    Approve,
    /// Deny this call
    Deny,
    /// Approve and auto-approve future calls
    AlwaysApprove,
    /// Deny and signal to quit
    Quit,
}

#[async_trait]
impl PermissionHandler for CliPermissionHandler {
    async fn request_approval(&self, call: &ToolCall, tool: &ToolDefinition) -> bool {
        // Check if already always approved
        if self.is_always_approved(&call.name) {
            eprintln!("[Tool: {}] Auto-approved", call.name);
            return true;
        }

        // Clone the data we need for the blocking thread
        let tool_name = call.name.clone();
        let tool_desc = tool.description.clone();
        let is_destructive = tool.is_destructive;
        let args = call.arguments.clone();

        // Use spawn_blocking for stdin read since it's blocking I/O
        let response = tokio::task::spawn_blocking(move || {
            Self::prompt_user_sync(&tool_name, &tool_desc, is_destructive, &args)
        })
        .await
        .unwrap_or(ApprovalResponse::Deny);

        match response {
            ApprovalResponse::Approve => true,
            ApprovalResponse::AlwaysApprove => {
                self.set_always_approved(&call.name);
                true
            }
            ApprovalResponse::Deny | ApprovalResponse::Quit => false,
        }
    }

    fn on_execute(&self, call: &ToolCall) {
        eprintln!("[Tool: {}] Executing...", call.name);
    }

    fn on_complete(&self, call: &ToolCall, result: &ToolResult) {
        let status = if result.success { "completed" } else { "failed" };
        eprintln!(
            "[Tool: {}] {} ({}ms)",
            call.name, status, result.duration_ms
        );
    }
}

/// Truncate a string to a maximum length, adding ellipsis if needed.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len > 3 {
        format!("{}...", &s[..max_len - 3])
    } else {
        s[..max_len].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 8), "hello...");
        assert_eq!(truncate("hi", 2), "hi");
    }

    #[test]
    fn test_always_approved() {
        let handler = CliPermissionHandler::new();
        assert!(!handler.is_always_approved("Read"));

        handler.set_always_approved("Read");
        assert!(handler.is_always_approved("Read"));
        assert!(!handler.is_always_approved("Write"));
    }

    #[test]
    fn test_cli_event_handler_creation() {
        let handler = CliEventHandler::new();
        assert!(handler.tool_start.lock().unwrap().is_none());
    }
}
