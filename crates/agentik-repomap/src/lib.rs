//! # agentik-repomap
//!
//! Repository mapping and code analysis for Agentik.
//!
//! This crate provides:
//! - Tree-sitter based multi-language parsing
//! - Symbol extraction (functions, classes, types)
//! - Dependency graph construction
//! - PageRank-based file ranking
//! - Context-aware serialization

pub mod graph;
pub mod parser;
pub mod ranking;
pub mod serializer;

pub use graph::DependencyGraph;
pub use parser::TreeSitterParser;
pub use ranking::PageRankScorer;
pub use serializer::RepoMapSerializer;
