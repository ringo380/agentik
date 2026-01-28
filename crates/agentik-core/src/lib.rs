//! # agentik-core
//!
//! Core types and abstractions for Agentik - the CLI-based agentic AI tool.
//!
//! This crate provides:
//! - Message and conversation primitives
//! - Tool definitions and execution types
//! - Session and state management types
//! - Configuration system
//! - Common error types

pub mod config;
pub mod error;
pub mod message;
pub mod session;
pub mod tool;

pub use config::Config;
pub use error::{Error, Result};
pub use message::{Content, Message, Role};
pub use session::{Session, SessionMetadata, SessionState};
pub use tool::{ToolCall, ToolDefinition, ToolResult};
