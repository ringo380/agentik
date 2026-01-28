//! # agentik-tools
//!
//! Built-in tool implementations for Agentik.
//!
//! This crate provides:
//! - File operations (read, write, edit, glob, grep)
//! - Shell execution (sandboxed)
//! - Git operations
//! - Web fetch and search

pub mod external;
pub mod file_ops;
pub mod git;
pub mod registry;
pub mod shell;
pub mod web;

pub use registry::ToolRegistry;
