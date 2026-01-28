//! # agentik-session
//!
//! Session persistence and context management for Agentik.
//!
//! This crate provides:
//! - SQLite-backed session storage with JSONL message logs
//! - Session lifecycle management (create, resume, archive)
//! - Context window management and token tracking
//! - Automatic compaction and summarization
//! - Session recovery with prefix matching
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use agentik_session::{
//!     store::SqliteSessionStore,
//!     recovery::SessionRecovery,
//! };
//!
//! // Open the default session store
//! let store = SqliteSessionStore::open_default()?;
//! let recovery = SessionRecovery::new(store);
//!
//! // Resume the most recent session
//! let session = recovery.resume_most_recent().await?;
//!
//! // Or resume by ID prefix
//! let session = recovery.resume_by_prefix("abc123").await?;
//! ```
//!
//! ## Storage Architecture
//!
//! Sessions are stored in:
//! - `~/.local/share/agentik/sessions.db` - SQLite database for metadata
//! - `~/.local/share/agentik/sessions/{id}/messages.jsonl` - Append-only message logs
//!
//! ## Context Management
//!
//! The [`context::ContextManager`] tracks token usage and determines when
//! compaction is needed. When the context window approaches capacity, the
//! [`compaction::Compactor`] generates summaries of older messages.

pub mod compaction;
pub mod context;
pub mod recovery;
pub mod store;

// Re-export commonly used types
pub use compaction::{
    CompactionConfig, CompletionProvider, Compactor, ExtractionResult,
    LlmSummaryConfig, LlmSummaryGenerator, SimpleSummaryGenerator, SummaryGenerator,
};
pub use context::{
    AdditionEstimate, CompactionBoundary, ContextConfig, ContextManager, ContextUsage,
    PreparedContext,
};
pub use recovery::{
    IssueSeverity, RecoveryError, RecoveryOptions, SessionRecovery, ValidationIssue,
    ValidationResult,
};
pub use store::{
    AppendResult, SessionQuery, SessionStore, SessionSummary, SqliteSessionStore, StoreError,
};
