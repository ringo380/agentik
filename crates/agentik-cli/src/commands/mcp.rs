//! MCP server management commands.

use crate::McpAction;

pub async fn handle(action: McpAction) -> anyhow::Result<()> {
    match action {
        McpAction::List => {
            println!("Configured MCP servers:");
            // TODO: List MCP servers
        }
        McpAction::Add { name, target } => {
            println!("Adding MCP server: {} -> {}", name, target);
            // TODO: Add MCP server
        }
        McpAction::Remove { name } => {
            println!("Removing MCP server: {}", name);
            // TODO: Remove MCP server
        }
        McpAction::Enable { name } => {
            println!("Enabling MCP server: {}", name);
            // TODO: Enable MCP server
        }
        McpAction::Disable { name } => {
            println!("Disabling MCP server: {}", name);
            // TODO: Disable MCP server
        }
        McpAction::Logs { name } => {
            println!("Logs for MCP server: {}", name);
            // TODO: Show MCP server logs
        }
    }
    Ok(())
}
