//! # agentik-mcp
//!
//! MCP (Model Context Protocol) client integration for Agentik.
//!
//! This crate provides:
//! - MCP client for connecting to servers
//! - stdio and HTTP transport support
//! - Tool discovery and registration
//! - Server lifecycle management

pub mod client;
pub mod discovery;
pub mod protocol;
pub mod transport;

pub use client::McpClient;
pub use discovery::McpServerManager;
