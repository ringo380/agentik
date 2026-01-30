//! Session and state management types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Session state in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    /// Session is active and accepting input
    Active,
    /// Session is being compacted
    Compacting,
    /// Session is suspended (waiting for input)
    Suspended,
    /// Session is sleeping (backgrounded)
    Sleeping,
    /// Session is archived (completed)
    Archived,
}

/// Session metadata for indexing and display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    /// Unique session ID
    pub id: String,
    /// Schema version for migrations
    pub version: u32,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
    /// Last activity timestamp
    pub last_active_at: DateTime<Utc>,
    /// Current session state
    pub state: SessionState,
    /// Working directory
    pub working_directory: PathBuf,
    /// User-provided or auto-generated title
    pub title: Option<String>,
    /// Tags for organization
    #[serde(default)]
    pub tags: Vec<String>,
    /// Files explicitly added to context
    #[serde(default)]
    pub added_files: Vec<PathBuf>,
    /// Parent session ID (for forked sessions)
    pub parent_session_id: Option<String>,
    /// Git context
    pub git: Option<GitContext>,
    /// Usage metrics
    pub metrics: SessionMetrics,
    /// Model configuration
    pub model: ModelConfig,
}

impl SessionMetadata {
    /// Create a new session with default values.
    pub fn new(working_directory: PathBuf) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            version: 1,
            created_at: now,
            updated_at: now,
            last_active_at: now,
            state: SessionState::Active,
            working_directory,
            title: None,
            tags: vec![],
            added_files: vec![],
            parent_session_id: None,
            git: None,
            metrics: SessionMetrics::default(),
            model: ModelConfig::default(),
        }
    }
}

/// Git repository context for a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitContext {
    /// Repository root path
    pub repository: PathBuf,
    /// Current branch
    pub branch: String,
    /// Commit SHA at session start
    pub commit_at_start: String,
    /// Commit SHA when archived
    pub commit_at_end: Option<String>,
    /// Files that were dirty at session start
    #[serde(default)]
    pub dirty_files: Vec<String>,
}

/// Usage metrics for a session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionMetrics {
    /// Total input tokens
    pub total_tokens_in: u64,
    /// Total output tokens
    pub total_tokens_out: u64,
    /// Total cost (USD)
    pub total_cost: f64,
    /// Number of conversation turns
    pub turn_count: u32,
    /// Number of compactions performed
    pub compaction_count: u32,
    /// Number of tool calls made
    pub tool_calls: u32,
}

/// Model configuration for a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Provider name
    pub provider: String,
    /// Model ID
    pub model_id: String,
    /// Temperature setting
    pub temperature: f32,
    /// Max tokens per response
    pub max_tokens: u32,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".to_string(),
            model_id: "claude-sonnet-4-20250514".to_string(),
            temperature: 0.7,
            max_tokens: 8192,
        }
    }
}

/// Full session including conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Session metadata
    pub metadata: SessionMetadata,
    /// Conversation messages
    pub messages: Vec<super::Message>,
    /// Compacted summary (if compaction has occurred)
    pub summary: Option<CompactedSummary>,
    /// Index of first non-compacted message
    pub compact_boundary: usize,
}

impl Session {
    /// Create a new session.
    pub fn new(working_directory: PathBuf) -> Self {
        Self {
            metadata: SessionMetadata::new(working_directory),
            messages: vec![],
            summary: None,
            compact_boundary: 0,
        }
    }

    /// Get the session ID.
    pub fn id(&self) -> &str {
        &self.metadata.id
    }

    /// Add a message to the session.
    pub fn add_message(&mut self, message: super::Message) {
        self.messages.push(message);
        self.metadata.metrics.turn_count += 1;
        self.metadata.updated_at = Utc::now();
        self.metadata.last_active_at = Utc::now();
    }
}

/// Result of context compaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactedSummary {
    /// Summary text
    pub text: String,
    /// Key decisions made
    pub key_decisions: Vec<String>,
    /// Files modified
    pub modified_files: Vec<PathBuf>,
    /// When compaction occurred
    pub created_at: DateTime<Utc>,
    /// Number of messages compacted
    pub messages_compacted: u32,
}
