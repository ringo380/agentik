//! Context window management.
//!
//! Manages the context window for LLM conversations, tracking token usage
//! and determining when compaction is needed.

use serde::{Deserialize, Serialize};

use agentik_core::{session::CompactedSummary, Message, Session};

/// Configuration for context window management.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    /// Maximum context tokens for the model.
    pub max_context_tokens: u32,
    /// Threshold (0.0-1.0) at which to trigger compaction.
    pub compaction_threshold: f32,
    /// Minimum tokens to preserve for recent messages.
    pub min_recent_tokens: u32,
    /// Minimum number of recent messages to always preserve.
    pub preserve_recent_messages: usize,
    /// Average characters per token (for estimation).
    pub chars_per_token: f32,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: 200_000,      // Claude's context window
            compaction_threshold: 0.80,        // Trigger at 80%
            min_recent_tokens: 20_000,         // Reserve 20K for recent context
            preserve_recent_messages: 10,      // Always keep last 10 messages
            chars_per_token: 4.0,              // Rough estimate
        }
    }
}

impl ContextConfig {
    /// Create config for a smaller context window (e.g., GPT-4).
    pub fn small() -> Self {
        Self {
            max_context_tokens: 8_000,
            compaction_threshold: 0.75,
            min_recent_tokens: 2_000,
            preserve_recent_messages: 6,
            chars_per_token: 4.0,
        }
    }

    /// Create config for medium context window (e.g., GPT-4 Turbo).
    pub fn medium() -> Self {
        Self {
            max_context_tokens: 128_000,
            compaction_threshold: 0.80,
            min_recent_tokens: 15_000,
            preserve_recent_messages: 8,
            chars_per_token: 4.0,
        }
    }

    /// Threshold token count that triggers compaction.
    pub fn compaction_trigger(&self) -> u32 {
        (self.max_context_tokens as f32 * self.compaction_threshold) as u32
    }
}

/// Current context usage statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextUsage {
    /// Total tokens in current context.
    pub total_tokens: u32,
    /// Tokens in compacted summary.
    pub summary_tokens: u32,
    /// Tokens in preserved messages.
    pub message_tokens: u32,
    /// Number of messages in context.
    pub message_count: usize,
    /// Number of compacted messages.
    pub compacted_count: usize,
    /// Percentage of context used.
    pub usage_percent: f32,
    /// Whether compaction is needed.
    pub needs_compaction: bool,
}

/// Context prepared for sending to the LLM.
#[derive(Debug, Clone)]
pub struct PreparedContext {
    /// System message with summary prefix (if compacted).
    pub system_message: Option<String>,
    /// Messages to include in context.
    pub messages: Vec<Message>,
    /// Token count estimate.
    pub estimated_tokens: u32,
}

/// Manager for context window operations.
pub struct ContextManager {
    config: ContextConfig,
}

impl ContextManager {
    /// Create a new context manager with default config.
    pub fn new() -> Self {
        Self {
            config: ContextConfig::default(),
        }
    }

    /// Create a context manager with custom config.
    pub fn with_config(config: ContextConfig) -> Self {
        Self { config }
    }

    /// Get the configuration.
    pub fn config(&self) -> &ContextConfig {
        &self.config
    }

    /// Count tokens in a message (estimate if not stored).
    pub fn count_message_tokens(&self, message: &Message) -> u32 {
        if let Some(count) = message.token_count {
            return count;
        }

        // Estimate based on character count
        let text = message.content.as_text();
        let chars = text.len() as f32;
        (chars / self.config.chars_per_token) as u32
    }

    /// Count total tokens in a slice of messages.
    pub fn count_tokens(&self, messages: &[Message]) -> u32 {
        messages.iter().map(|m| self.count_message_tokens(m)).sum()
    }

    /// Count tokens in a summary.
    pub fn count_summary_tokens(&self, summary: &CompactedSummary) -> u32 {
        let text_chars = summary.text.len() as f32;
        let decisions_chars: f32 = summary
            .key_decisions
            .iter()
            .map(|d| d.len() as f32)
            .sum();
        let files_chars: f32 = summary
            .modified_files
            .iter()
            .map(|f| f.to_string_lossy().len() as f32)
            .sum();

        let total_chars = text_chars + decisions_chars + files_chars;
        (total_chars / self.config.chars_per_token) as u32
    }

    /// Check if a session needs compaction.
    pub fn needs_compaction(&self, session: &Session) -> bool {
        let usage = self.calculate_usage(session);
        usage.needs_compaction
    }

    /// Calculate current context usage for a session.
    pub fn calculate_usage(&self, session: &Session) -> ContextUsage {
        let summary_tokens = session
            .summary
            .as_ref()
            .map(|s| self.count_summary_tokens(s))
            .unwrap_or(0);

        let messages_after_boundary = &session.messages[session.compact_boundary..];
        let message_tokens = self.count_tokens(messages_after_boundary);
        let total_tokens = summary_tokens + message_tokens;

        let usage_percent = total_tokens as f32 / self.config.max_context_tokens as f32;
        let needs_compaction = total_tokens > self.config.compaction_trigger();

        ContextUsage {
            total_tokens,
            summary_tokens,
            message_tokens,
            message_count: messages_after_boundary.len(),
            compacted_count: session.compact_boundary,
            usage_percent,
            needs_compaction,
        }
    }

    /// Prepare context for sending to the LLM.
    ///
    /// Returns messages with an optional system message prefix containing
    /// the compacted summary.
    pub fn prepare_context(&self, session: &Session, base_system: Option<&str>) -> PreparedContext {
        let mut system_parts = Vec::new();

        // Add base system message if provided
        if let Some(base) = base_system {
            system_parts.push(base.to_string());
        }

        // Add compacted summary if present
        if let Some(ref summary) = session.summary {
            system_parts.push(self.format_summary(summary));
        }

        let system_message = if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n\n"))
        };

        // Get messages after compaction boundary
        let messages: Vec<Message> = session.messages[session.compact_boundary..]
            .iter()
            .cloned()
            .collect();

        let estimated_tokens = self.count_tokens(&messages)
            + session
                .summary
                .as_ref()
                .map(|s| self.count_summary_tokens(s))
                .unwrap_or(0);

        PreparedContext {
            system_message,
            messages,
            estimated_tokens,
        }
    }

    /// Format a compacted summary for inclusion in system message.
    fn format_summary(&self, summary: &CompactedSummary) -> String {
        let mut parts = Vec::new();

        parts.push("## Previous Conversation Summary".to_string());
        parts.push(summary.text.clone());

        if !summary.key_decisions.is_empty() {
            parts.push("\n### Key Decisions Made".to_string());
            for decision in &summary.key_decisions {
                parts.push(format!("- {}", decision));
            }
        }

        if !summary.modified_files.is_empty() {
            parts.push("\n### Files Modified".to_string());
            for file in &summary.modified_files {
                parts.push(format!("- `{}`", file.display()));
            }
        }

        parts.join("\n")
    }

    /// Find the optimal compaction boundary.
    ///
    /// Returns the index in the messages array where compaction should end,
    /// preserving at least `preserve_recent_messages` and `min_recent_tokens`.
    pub fn find_compaction_boundary(&self, session: &Session) -> CompactionBoundary {
        let messages = &session.messages;
        let total_count = messages.len();

        // Nothing to compact if we have few messages
        if total_count <= self.config.preserve_recent_messages {
            return CompactionBoundary {
                index: 0,
                messages_to_compact: 0,
                tokens_to_compact: 0,
                preserved_count: total_count,
                preserved_tokens: self.count_tokens(messages),
            };
        }

        // Find the boundary that:
        // 1. Keeps at least preserve_recent_messages
        // 2. Keeps at least min_recent_tokens
        // 3. Compacts as much as possible while respecting 1 & 2

        let min_preserve = self.config.preserve_recent_messages;
        let min_tokens = self.config.min_recent_tokens;

        // Start from the end and work backward
        let mut preserved_tokens: u32 = 0;
        let mut boundary = total_count;

        for i in (0..total_count).rev() {
            let msg_tokens = self.count_message_tokens(&messages[i]);
            preserved_tokens += msg_tokens;

            let preserved_count = total_count - i;

            // Check if we've preserved enough
            if preserved_count >= min_preserve && preserved_tokens >= min_tokens {
                boundary = i;
                break;
            }

            boundary = i;
        }

        // Don't compact past the current boundary
        let effective_boundary = boundary.max(session.compact_boundary);

        let tokens_to_compact = self.count_tokens(&messages[session.compact_boundary..effective_boundary]);
        let messages_to_compact = effective_boundary - session.compact_boundary;

        CompactionBoundary {
            index: effective_boundary,
            messages_to_compact,
            tokens_to_compact,
            preserved_count: total_count - effective_boundary,
            preserved_tokens,
        }
    }

    /// Estimate tokens needed for a new message before adding it.
    pub fn estimate_addition(&self, session: &Session, content: &str) -> AdditionEstimate {
        let current_usage = self.calculate_usage(session);
        let new_tokens = (content.len() as f32 / self.config.chars_per_token) as u32;
        let total_after = current_usage.total_tokens + new_tokens;
        let usage_after = total_after as f32 / self.config.max_context_tokens as f32;

        AdditionEstimate {
            current_tokens: current_usage.total_tokens,
            added_tokens: new_tokens,
            total_tokens: total_after,
            usage_percent: usage_after,
            triggers_compaction: total_after > self.config.compaction_trigger(),
            exceeds_limit: total_after > self.config.max_context_tokens,
        }
    }
}

impl Default for ContextManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of finding a compaction boundary.
#[derive(Debug, Clone)]
pub struct CompactionBoundary {
    /// Index where preserved messages start.
    pub index: usize,
    /// Number of messages to compact.
    pub messages_to_compact: usize,
    /// Tokens in messages to compact.
    pub tokens_to_compact: u32,
    /// Number of messages preserved.
    pub preserved_count: usize,
    /// Tokens in preserved messages.
    pub preserved_tokens: u32,
}

/// Estimate of adding new content to context.
#[derive(Debug, Clone)]
pub struct AdditionEstimate {
    /// Current token count.
    pub current_tokens: u32,
    /// Tokens to be added.
    pub added_tokens: u32,
    /// Total tokens after addition.
    pub total_tokens: u32,
    /// Usage percentage after addition.
    pub usage_percent: f32,
    /// Whether this triggers compaction.
    pub triggers_compaction: bool,
    /// Whether this exceeds the limit.
    pub exceeds_limit: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn create_test_session() -> Session {
        let mut session = Session::new(PathBuf::from("/tmp/test"));

        // Add some messages
        for i in 0..20 {
            let msg = if i % 2 == 0 {
                Message::user(format!("User message {}", i))
            } else {
                Message::assistant(format!("Assistant response {} with some longer content to simulate real responses", i))
            };
            session.messages.push(msg);
        }

        session
    }

    #[test]
    fn test_count_tokens() {
        let manager = ContextManager::new();
        let msg = Message::user("Hello, how are you today?");

        // With default 4 chars/token: 27 chars / 4 = 6.75 -> 6 tokens
        let tokens = manager.count_message_tokens(&msg);
        assert!(tokens > 0);
    }

    #[test]
    fn test_context_usage() {
        let manager = ContextManager::new();
        let session = create_test_session();

        let usage = manager.calculate_usage(&session);
        assert!(usage.total_tokens > 0);
        assert_eq!(usage.message_count, 20);
        assert_eq!(usage.compacted_count, 0);
    }

    #[test]
    fn test_needs_compaction_small_session() {
        let manager = ContextManager::new();
        let session = create_test_session();

        // Small session shouldn't need compaction
        assert!(!manager.needs_compaction(&session));
    }

    #[test]
    fn test_find_compaction_boundary() {
        let manager = ContextManager::with_config(ContextConfig {
            preserve_recent_messages: 5,
            min_recent_tokens: 50,
            ..Default::default()
        });

        let session = create_test_session();
        let boundary = manager.find_compaction_boundary(&session);

        // Should preserve at least 5 messages
        assert!(boundary.preserved_count >= 5);
        assert!(boundary.index <= 15); // 20 - 5 = 15
    }

    #[test]
    fn test_prepare_context() {
        let manager = ContextManager::new();
        let session = create_test_session();

        let context = manager.prepare_context(&session, Some("You are a helpful assistant."));

        assert!(context.system_message.is_some());
        assert_eq!(context.messages.len(), 20);
    }

    #[test]
    fn test_estimate_addition() {
        let manager = ContextManager::new();
        let session = create_test_session();

        let estimate = manager.estimate_addition(&session, "A short message");

        assert!(estimate.added_tokens > 0);
        assert!(estimate.total_tokens > estimate.current_tokens);
        assert!(!estimate.exceeds_limit); // Small addition shouldn't exceed
    }
}
