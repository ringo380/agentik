//! Integration tests for MCP client with real MCP servers.

use agentik_mcp::{McpClient, McpServerConfig, McpServerManager};
use serde_json::json;
use std::sync::Arc;

/// Test connecting to the filesystem MCP server.
#[tokio::test]
async fn test_filesystem_server() {
    // Create a server config for the filesystem MCP server
    let config = McpServerConfig::new("filesystem", "npx")
        .with_args(vec![
            "-y".to_string(),
            "@modelcontextprotocol/server-filesystem".to_string(),
            "/private/tmp".to_string(), // Allow access to /tmp (use /private/tmp on macOS)
        ]);

    let client = Arc::new(McpClient::new());

    // Connect to the server
    let connection = client.connect(config).await;

    match connection {
        Ok(conn) => {
            println!("Connected to filesystem server");

            // List tools
            let tools = conn.list_tools().await.expect("Failed to list tools");
            println!("Available tools: {:?}", tools.iter().map(|t| &t.name).collect::<Vec<_>>());

            assert!(!tools.is_empty(), "Server should have at least one tool");

            // Check for expected tools
            let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
            println!("Tool names: {:?}", tool_names);

            // The filesystem server should have read_file, write_file, etc.
            assert!(
                tool_names.contains(&"read_file") || tool_names.contains(&"list_directory"),
                "Expected filesystem tools"
            );

            // Try calling list_directory on /private/tmp (macOS resolves /tmp to this)
            if tool_names.contains(&"list_directory") {
                let result = conn
                    .call_tool("list_directory", Some(json!({"path": "/private/tmp"})))
                    .await
                    .expect("Failed to call list_directory");

                println!("list_directory result: {:?}", result);
                assert!(!result.is_error, "list_directory should succeed");

                // Verify we got some content back
                assert!(!result.content.is_empty(), "Should have content");
            }

            // Close the connection
            conn.close().await.expect("Failed to close connection");
            println!("Connection closed successfully");
        }
        Err(e) => {
            // If npx or the package isn't available, skip the test
            eprintln!("Could not connect to filesystem server: {}", e);
            eprintln!("This may be expected if the MCP server package is not installed.");
        }
    }
}

/// Test the server manager with multiple servers.
#[tokio::test]
async fn test_server_manager() {
    let manager = McpServerManager::new();

    // Add a server config
    let config = McpServerConfig::new("filesystem", "npx")
        .with_args(vec![
            "-y".to_string(),
            "@modelcontextprotocol/server-filesystem".to_string(),
            "/tmp".to_string(),
        ]);

    manager.add_config(config).await;

    // Start the server
    match manager.start_server("filesystem").await {
        Ok(()) => {
            println!("Server started via manager");

            // Check it's connected
            assert!(manager.is_connected("filesystem").await);

            // Get status
            let status = manager.server_status("filesystem").await;
            println!("Server status: {}", status);

            // Stop the server
            manager.stop_server("filesystem").await.expect("Failed to stop server");
            assert!(!manager.is_connected("filesystem").await);

            println!("Server stopped successfully");
        }
        Err(e) => {
            eprintln!("Could not start server via manager: {}", e);
        }
    }
}

/// Test tool wrapper integration.
#[tokio::test]
async fn test_tool_wrapper() {
    use agentik_mcp::McpToolWrapper;
    use agentik_tools::ToolRegistry;

    let manager = McpServerManager::new();

    let config = McpServerConfig::new("filesystem", "npx")
        .with_args(vec![
            "-y".to_string(),
            "@modelcontextprotocol/server-filesystem".to_string(),
            "/tmp".to_string(),
        ]);

    manager.add_config(config).await;

    match manager.start_server("filesystem").await {
        Ok(()) => {
            // Create a tool registry and register MCP tools
            let mut registry = ToolRegistry::new();
            manager.register_tools(&mut registry).await;

            // Check that tools were registered
            let tool_names = registry.list();
            println!("Registered tools: {:?}", tool_names);

            // Find an MCP tool
            let mcp_tools: Vec<_> = tool_names
                .iter()
                .filter(|name| name.starts_with("mcp__"))
                .collect();

            println!("MCP tools: {:?}", mcp_tools);
            assert!(!mcp_tools.is_empty(), "Should have registered MCP tools");

            // Verify tool name parsing
            for tool_name in &mcp_tools {
                let parsed = McpToolWrapper::parse_tool_name(tool_name);
                assert!(parsed.is_some(), "Should parse MCP tool name: {}", tool_name);
                let (server, tool) = parsed.unwrap();
                assert_eq!(server, "filesystem");
                println!("  {} -> server={}, tool={}", tool_name, server, tool);
            }

            manager.stop_all().await.expect("Failed to stop servers");
        }
        Err(e) => {
            eprintln!("Could not start server for tool wrapper test: {}", e);
        }
    }
}
