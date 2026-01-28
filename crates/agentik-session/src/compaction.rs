//! Context compaction and summarization.
//!
//! Provides algorithms for compacting conversation history into summaries
//! while preserving key information like decisions and file modifications.

use std::collections::HashSet;
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use agentik_core::{session::CompactedSummary, Message, Role, Session};

use crate::context::{CompactionBoundary, ContextManager};

/// Configuration for the compactor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Temperature for summary generation (lower = more focused).
    pub temperature: f32,
    /// Maximum tokens for the summary.
    pub max_summary_tokens: u32,
    /// Whether to extract file modifications.
    pub extract_files: bool,
    /// Whether to extract key decisions.
    pub extract_decisions: bool,
    /// Tool names that modify files.
    pub file_modifying_tools: Vec<String>,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            temperature: 0.3,
            max_summary_tokens: 2000,
            extract_files: true,
            extract_decisions: true,
            file_modifying_tools: vec![
                "Write".to_string(),
                "Edit".to_string(),
                "Bash".to_string(),
                "write_file".to_string(),
                "edit_file".to_string(),
                "create_file".to_string(),
            ],
        }
    }
}

/// Result of extracting information from messages.
#[derive(Debug, Clone, Default)]
pub struct ExtractionResult {
    /// Files that were modified.
    pub modified_files: Vec<PathBuf>,
    /// Key decisions or conclusions.
    pub key_decisions: Vec<String>,
    /// User goals/requests extracted.
    pub user_goals: Vec<String>,
    /// Summary of tool usage.
    pub tool_summary: Vec<String>,
}

/// Trait for generating summaries (allows mocking in tests).
#[async_trait::async_trait]
pub trait SummaryGenerator: Send + Sync {
    /// Generate a summary of the given messages.
    async fn generate_summary(
        &self,
        messages: &[Message],
        extraction: &ExtractionResult,
        previous_summary: Option<&CompactedSummary>,
    ) -> anyhow::Result<String>;
}

/// Compactor for session context.
pub struct Compactor {
    config: CompactionConfig,
    context_manager: ContextManager,
}

impl Compactor {
    /// Create a new compactor with default configuration.
    pub fn new() -> Self {
        Self {
            config: CompactionConfig::default(),
            context_manager: ContextManager::new(),
        }
    }

    /// Create a compactor with custom configuration.
    pub fn with_config(config: CompactionConfig, context_manager: ContextManager) -> Self {
        Self {
            config,
            context_manager,
        }
    }

    /// Get the compaction configuration.
    pub fn config(&self) -> &CompactionConfig {
        &self.config
    }

    /// Extract information from messages that should be preserved.
    pub fn extract_information(&self, messages: &[Message]) -> ExtractionResult {
        let mut result = ExtractionResult::default();
        let mut seen_files: HashSet<PathBuf> = HashSet::new();
        let mut seen_decisions: HashSet<String> = HashSet::new();

        for message in messages {
            match message.role {
                Role::User => {
                    // Extract user goals from user messages
                    if let Some(goal) = self.extract_user_goal(message) {
                        result.user_goals.push(goal);
                    }
                }
                Role::Assistant => {
                    // Extract decisions from assistant messages
                    if self.config.extract_decisions {
                        for decision in self.extract_decisions(message) {
                            if !seen_decisions.contains(&decision) {
                                seen_decisions.insert(decision.clone());
                                result.key_decisions.push(decision);
                            }
                        }
                    }

                    // Extract file modifications from tool calls
                    if self.config.extract_files {
                        for file in self.extract_file_modifications(message) {
                            if !seen_files.contains(&file) {
                                seen_files.insert(file.clone());
                                result.modified_files.push(file);
                            }
                        }
                    }

                    // Summarize tool usage
                    for summary in self.summarize_tool_calls(message) {
                        result.tool_summary.push(summary);
                    }
                }
                _ => {}
            }
        }

        result
    }

    /// Extract a user goal from a user message.
    fn extract_user_goal(&self, message: &Message) -> Option<String> {
        let text = message.content.as_text();

        // Skip very short messages
        if text.len() < 10 {
            return None;
        }

        // Truncate long messages
        let goal = if text.len() > 200 {
            format!("{}...", &text[..197])
        } else {
            text
        };

        Some(goal)
    }

    /// Extract key decisions from an assistant message.
    fn extract_decisions(&self, message: &Message) -> Vec<String> {
        let mut decisions = Vec::new();
        let text = message.content.as_text();

        // Look for decision indicators
        let decision_patterns = [
            "I'll ",
            "I will ",
            "Let's ",
            "We should ",
            "The best approach ",
            "I've decided ",
            "I recommend ",
            "The solution is ",
        ];

        for line in text.lines() {
            let line = line.trim();
            for pattern in &decision_patterns {
                if line.starts_with(pattern) && line.len() > 20 && line.len() < 200 {
                    decisions.push(line.to_string());
                    break;
                }
            }
        }

        // Limit to most important decisions
        decisions.truncate(5);
        decisions
    }

    /// Extract file modifications from tool calls.
    fn extract_file_modifications(&self, message: &Message) -> Vec<PathBuf> {
        let mut files = Vec::new();

        for tool_call in &message.tool_calls {
            if self
                .config
                .file_modifying_tools
                .iter()
                .any(|t| t.eq_ignore_ascii_case(&tool_call.name))
            {
                // Try to extract file path from arguments
                if let Some(path) = self.extract_path_from_args(&tool_call.arguments) {
                    files.push(path);
                }
            }
        }

        files
    }

    /// Extract file path from tool arguments.
    fn extract_path_from_args(&self, args: &serde_json::Value) -> Option<PathBuf> {
        // Common argument names for file paths
        let path_keys = ["file_path", "path", "file", "filename", "target"];

        if let Some(obj) = args.as_object() {
            for key in &path_keys {
                if let Some(value) = obj.get(*key) {
                    if let Some(path_str) = value.as_str() {
                        return Some(PathBuf::from(path_str));
                    }
                }
            }
        }

        None
    }

    /// Summarize tool calls in a message.
    fn summarize_tool_calls(&self, message: &Message) -> Vec<String> {
        message
            .tool_calls
            .iter()
            .map(|tc| {
                let args_summary = self.summarize_args(&tc.arguments);
                format!("{}: {}", tc.name, args_summary)
            })
            .collect()
    }

    /// Create a brief summary of tool arguments.
    fn summarize_args(&self, args: &serde_json::Value) -> String {
        match args {
            serde_json::Value::Object(obj) => {
                let keys: Vec<&str> = obj.keys().map(|s| s.as_str()).take(3).collect();
                if keys.is_empty() {
                    "()".to_string()
                } else {
                    format!("({})", keys.join(", "))
                }
            }
            _ => "()".to_string(),
        }
    }

    /// Find the compaction boundary for a session.
    pub fn find_boundary(&self, session: &Session) -> CompactionBoundary {
        self.context_manager.find_compaction_boundary(session)
    }

    /// Create a compacted summary without using an LLM.
    ///
    /// This creates a structured summary based on extracted information.
    /// For full summarization, use `compact_with_generator`.
    pub fn compact_simple(&self, session: &Session) -> Option<CompactedSummary> {
        let boundary = self.find_boundary(session);

        if boundary.messages_to_compact == 0 {
            return None;
        }

        let messages_to_compact = &session.messages[session.compact_boundary..boundary.index];

        let extraction = self.extract_information(messages_to_compact);

        // Build a simple summary text
        let mut summary_parts = Vec::new();

        if !extraction.user_goals.is_empty() {
            summary_parts.push("User requested:".to_string());
            for goal in extraction.user_goals.iter().take(3) {
                summary_parts.push(format!("- {}", goal));
            }
        }

        if !extraction.tool_summary.is_empty() {
            summary_parts.push("\nActions taken:".to_string());
            for action in extraction.tool_summary.iter().take(10) {
                summary_parts.push(format!("- {}", action));
            }
        }

        let summary_text = if summary_parts.is_empty() {
            format!(
                "Compacted {} messages from the conversation.",
                boundary.messages_to_compact
            )
        } else {
            summary_parts.join("\n")
        };

        Some(CompactedSummary {
            text: summary_text,
            key_decisions: extraction.key_decisions,
            modified_files: extraction.modified_files,
            created_at: Utc::now(),
            messages_compacted: boundary.messages_to_compact as u32,
        })
    }

    /// Compact with a custom summary generator.
    pub async fn compact_with_generator<G: SummaryGenerator>(
        &self,
        session: &Session,
        generator: &G,
    ) -> anyhow::Result<Option<CompactedSummary>> {
        let boundary = self.find_boundary(session);

        if boundary.messages_to_compact == 0 {
            return Ok(None);
        }

        let messages_to_compact = &session.messages[session.compact_boundary..boundary.index];

        let extraction = self.extract_information(messages_to_compact);

        // Generate summary using the provided generator
        let summary_text = generator
            .generate_summary(messages_to_compact, &extraction, session.summary.as_ref())
            .await?;

        Ok(Some(CompactedSummary {
            text: summary_text,
            key_decisions: extraction.key_decisions,
            modified_files: extraction.modified_files,
            created_at: Utc::now(),
            messages_compacted: boundary.messages_to_compact as u32,
        }))
    }

    /// Merge a new summary with an existing one (for incremental compaction).
    pub fn merge_summaries(
        &self,
        previous: &CompactedSummary,
        new: &CompactedSummary,
    ) -> CompactedSummary {
        // Combine texts
        let combined_text = format!("{}\n\n---\n\n{}", previous.text, new.text);

        // Merge file lists (deduplicate)
        let mut all_files: HashSet<PathBuf> = HashSet::new();
        for file in &previous.modified_files {
            all_files.insert(file.clone());
        }
        for file in &new.modified_files {
            all_files.insert(file.clone());
        }

        // Merge decisions (keep recent ones prioritized)
        let mut all_decisions = new.key_decisions.clone();
        for decision in &previous.key_decisions {
            if !all_decisions.contains(decision) && all_decisions.len() < 10 {
                all_decisions.push(decision.clone());
            }
        }

        CompactedSummary {
            text: combined_text,
            key_decisions: all_decisions,
            modified_files: all_files.into_iter().collect(),
            created_at: Utc::now(),
            messages_compacted: previous.messages_compacted + new.messages_compacted,
        }
    }

    /// Build a prompt for LLM-based summary generation.
    pub fn build_summary_prompt(
        &self,
        messages: &[Message],
        extraction: &ExtractionResult,
        previous_summary: Option<&CompactedSummary>,
    ) -> String {
        let mut prompt_parts = Vec::new();

        prompt_parts.push(
            "Summarize the following conversation concisely, focusing on:
1. What the user wanted to accomplish
2. What actions were taken
3. The current state/outcome

Keep the summary under 500 words. Be factual and specific."
                .to_string(),
        );

        if let Some(prev) = previous_summary {
            prompt_parts.push(format!("\n## Previous Summary\n{}", prev.text));
        }

        prompt_parts.push("\n## Conversation to Summarize".to_string());

        for msg in messages {
            let role = match msg.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::System => "System",
                Role::Tool => "Tool Result",
            };
            let content = msg.content.as_text();
            let truncated = if content.len() > 500 {
                format!("{}...[truncated]", &content[..497])
            } else {
                content
            };
            prompt_parts.push(format!("\n**{}**: {}", role, truncated));
        }

        if !extraction.modified_files.is_empty() {
            prompt_parts.push("\n## Files Modified".to_string());
            for file in &extraction.modified_files {
                prompt_parts.push(format!("- {}", file.display()));
            }
        }

        prompt_parts.join("\n")
    }
}

impl Default for Compactor {
    fn default() -> Self {
        Self::new()
    }
}

/// A simple summary generator that just concatenates user goals.
pub struct SimpleSummaryGenerator;

#[async_trait::async_trait]
impl SummaryGenerator for SimpleSummaryGenerator {
    async fn generate_summary(
        &self,
        _messages: &[Message],
        extraction: &ExtractionResult,
        previous_summary: Option<&CompactedSummary>,
    ) -> anyhow::Result<String> {
        let mut parts = Vec::new();

        if let Some(prev) = previous_summary {
            parts.push(format!("Previously: {}", prev.text));
        }

        if !extraction.user_goals.is_empty() {
            parts.push("User goals:".to_string());
            for goal in &extraction.user_goals {
                parts.push(format!("- {}", goal));
            }
        }

        if !extraction.tool_summary.is_empty() {
            parts.push("Actions taken:".to_string());
            for action in extraction.tool_summary.iter().take(10) {
                parts.push(format!("- {}", action));
            }
        }

        Ok(parts.join("\n"))
    }
}

/// Configuration for LLM-based summary generation.
#[derive(Debug, Clone)]
pub struct LlmSummaryConfig {
    /// Model to use for summarization (prefer fast/cheap models)
    pub model: String,
    /// Maximum tokens for the generated summary
    pub max_tokens: u32,
    /// Temperature for generation (lower = more focused)
    pub temperature: f32,
    /// System prompt for the summarizer
    pub system_prompt: String,
}

impl Default for LlmSummaryConfig {
    fn default() -> Self {
        Self {
            model: "claude-haiku-3-5-20241022".to_string(),
            max_tokens: 1000,
            temperature: 0.3,
            system_prompt: "You are a precise summarizer. Create concise summaries that capture \
                            the essential information from conversations. Focus on: what was \
                            requested, what actions were taken, and the current state. Be factual \
                            and specific. Keep summaries under 500 words."
                .to_string(),
        }
    }
}

/// LLM-based summary generator that uses a provider.
///
/// This generator calls an LLM to create intelligent summaries of conversation
/// history, preserving context while reducing token usage.
///
/// # Example
///
/// ```rust,ignore
/// use agentik_session::compaction::{LlmSummaryGenerator, LlmSummaryConfig};
/// use agentik_providers::AnthropicProvider;
/// use std::sync::Arc;
///
/// let provider = Arc::new(AnthropicProvider::from_env().unwrap());
/// let generator = LlmSummaryGenerator::new(provider, LlmSummaryConfig::default());
///
/// // Use with Compactor::compact_with_generator
/// ```
pub struct LlmSummaryGenerator<P> {
    provider: std::sync::Arc<P>,
    config: LlmSummaryConfig,
    compaction_config: CompactionConfig,
}

impl<P> LlmSummaryGenerator<P> {
    /// Create a new LLM summary generator.
    pub fn new(provider: std::sync::Arc<P>, config: LlmSummaryConfig) -> Self {
        Self {
            provider,
            config,
            compaction_config: CompactionConfig::default(),
        }
    }

    /// Create with custom compaction config.
    pub fn with_compaction_config(mut self, config: CompactionConfig) -> Self {
        self.compaction_config = config;
        self
    }

    /// Build the summarization prompt.
    fn build_prompt(
        &self,
        messages: &[Message],
        extraction: &ExtractionResult,
        previous_summary: Option<&CompactedSummary>,
    ) -> String {
        let compactor =
            Compactor::with_config(self.compaction_config.clone(), ContextManager::new());
        compactor.build_summary_prompt(messages, extraction, previous_summary)
    }
}

/// Trait alias for providers that support completion.
pub trait CompletionProvider: Send + Sync {
    /// Generate a completion for the given prompt.
    fn complete_for_summary(
        &self,
        model: &str,
        system: &str,
        prompt: &str,
        max_tokens: u32,
        temperature: f32,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>>;
}

#[async_trait::async_trait]
impl<P: CompletionProvider> SummaryGenerator for LlmSummaryGenerator<P> {
    async fn generate_summary(
        &self,
        messages: &[Message],
        extraction: &ExtractionResult,
        previous_summary: Option<&CompactedSummary>,
    ) -> anyhow::Result<String> {
        let prompt = self.build_prompt(messages, extraction, previous_summary);

        // Call the provider to generate the summary
        let response = self
            .provider
            .complete_for_summary(
                &self.config.model,
                &self.config.system_prompt,
                &prompt,
                self.config.max_tokens,
                self.config.temperature,
            )
            .await?;

        // Post-process: ensure the summary isn't too long
        let summary = if response.len() > 2000 {
            format!("{}...", &response[..1997])
        } else {
            response
        };

        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentik_core::ToolCall;

    fn create_test_messages() -> Vec<Message> {
        vec![
            Message::user("Please create a new file called test.rs with a hello world function"),
            {
                let mut msg = Message::assistant("I'll create that file for you.");
                msg.tool_calls.push(ToolCall::new(
                    "call_1",
                    "Write",
                    serde_json::json!({
                        "file_path": "/tmp/test.rs",
                        "content": "fn hello() { println!(\"Hello!\"); }"
                    }),
                ));
                msg
            },
            Message::user("Great, now add a main function"),
            {
                let mut msg = Message::assistant("I'll add a main function to the file.");
                msg.tool_calls.push(ToolCall::new(
                    "call_2",
                    "Edit",
                    serde_json::json!({
                        "file_path": "/tmp/test.rs",
                        "old": "fn hello()",
                        "new": "fn main() { hello(); }\nfn hello()"
                    }),
                ));
                msg
            },
        ]
    }

    #[test]
    fn test_extract_file_modifications() {
        let compactor = Compactor::new();
        let messages = create_test_messages();

        let extraction = compactor.extract_information(&messages);

        assert!(!extraction.modified_files.is_empty());
        assert!(extraction
            .modified_files
            .contains(&PathBuf::from("/tmp/test.rs")));
    }

    #[test]
    fn test_extract_user_goals() {
        let compactor = Compactor::new();
        let messages = create_test_messages();

        let extraction = compactor.extract_information(&messages);

        assert!(!extraction.user_goals.is_empty());
        assert!(extraction.user_goals[0].contains("create a new file"));
    }

    #[test]
    fn test_extract_decisions() {
        let compactor = Compactor::new();
        let messages = create_test_messages();

        let extraction = compactor.extract_information(&messages);

        assert!(!extraction.key_decisions.is_empty());
    }

    #[test]
    fn test_compact_simple() {
        use crate::context::ContextConfig;

        // Use a config with low thresholds for testing
        let context_config = ContextConfig {
            preserve_recent_messages: 5,
            min_recent_tokens: 10, // Low threshold for testing
            ..Default::default()
        };
        let context_manager = crate::context::ContextManager::with_config(context_config);
        let compactor = Compactor::with_config(CompactionConfig::default(), context_manager);

        let mut session = Session::new(PathBuf::from("/tmp/test"));

        // Add enough messages to trigger compaction consideration
        for i in 0..25 {
            session
                .messages
                .push(Message::user(format!("Message {} with some content", i)));
            session.messages.push(Message::assistant(format!(
                "Response {} with more content",
                i
            )));
        }

        // Simple compaction should work even without LLM
        let summary = compactor.compact_simple(&session);
        assert!(summary.is_some());
        assert!(summary.unwrap().messages_compacted > 0);
    }

    #[test]
    fn test_merge_summaries() {
        let compactor = Compactor::new();

        let prev = CompactedSummary {
            text: "Previous work".to_string(),
            key_decisions: vec!["Decision 1".to_string()],
            modified_files: vec![PathBuf::from("/tmp/a.rs")],
            created_at: Utc::now(),
            messages_compacted: 10,
        };

        let new = CompactedSummary {
            text: "New work".to_string(),
            key_decisions: vec!["Decision 2".to_string()],
            modified_files: vec![PathBuf::from("/tmp/b.rs")],
            created_at: Utc::now(),
            messages_compacted: 5,
        };

        let merged = compactor.merge_summaries(&prev, &new);

        assert!(merged.text.contains("Previous work"));
        assert!(merged.text.contains("New work"));
        assert_eq!(merged.messages_compacted, 15);
        assert_eq!(merged.modified_files.len(), 2);
        assert_eq!(merged.key_decisions.len(), 2);
    }

    #[test]
    fn test_build_summary_prompt() {
        let compactor = Compactor::new();
        let messages = create_test_messages();
        let extraction = compactor.extract_information(&messages);

        let prompt = compactor.build_summary_prompt(&messages, &extraction, None);

        assert!(prompt.contains("Summarize"));
        assert!(prompt.contains("User"));
        assert!(prompt.contains("Assistant"));
    }
}
