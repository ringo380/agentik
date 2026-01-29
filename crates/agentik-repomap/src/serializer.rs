//! Repository map serialization for prompt injection.

use std::path::PathBuf;

use crate::types::{FileInfo, RepoMap, Symbol, SymbolKind};

/// Configuration for serialization.
#[derive(Debug, Clone)]
pub struct SerializeConfig {
    /// Maximum number of tokens (approximate) to use
    pub token_budget: usize,
    /// Include file ranks in output
    pub include_ranks: bool,
    /// Include symbol signatures
    pub include_signatures: bool,
    /// Maximum files to include
    pub max_files: Option<usize>,
    /// Minimum rank to include (0.0 - 1.0)
    pub min_rank: f64,
}

impl Default for SerializeConfig {
    fn default() -> Self {
        Self {
            token_budget: 2000,
            include_ranks: true,
            include_signatures: true,
            max_files: None,
            min_rank: 0.0,
        }
    }
}

impl SerializeConfig {
    /// Create a config with a specific token budget.
    pub fn with_budget(budget: usize) -> Self {
        Self {
            token_budget: budget,
            ..Default::default()
        }
    }

    /// Set maximum files.
    pub fn max_files(mut self, max: usize) -> Self {
        self.max_files = Some(max);
        self
    }

    /// Set whether to include signatures.
    pub fn signatures(mut self, include: bool) -> Self {
        self.include_signatures = include;
        self
    }

    /// Set whether to include ranks.
    pub fn ranks(mut self, include: bool) -> Self {
        self.include_ranks = include;
        self
    }

    /// Set minimum rank threshold.
    pub fn min_rank(mut self, min: f64) -> Self {
        self.min_rank = min;
        self
    }
}

/// Repository map serializer.
///
/// Converts a repo map into a compact text format suitable for
/// inclusion in LLM prompts.
pub struct RepoMapSerializer;

impl RepoMapSerializer {
    /// Serialize a repo map for prompt injection.
    ///
    /// Format:
    /// ```text
    /// src/main.rs (0.15)
    ///   fn main()
    ///   struct Config
    /// src/lib.rs (0.12)
    ///   fn process(input: &str) -> Result<()>
    ///   trait Handler
    /// ```
    pub fn serialize_for_prompt(map: &RepoMap, config: &SerializeConfig) -> String {
        let mut output = String::new();
        let mut token_count = 0;

        // Get files sorted by rank
        let files = map.files_by_rank();

        // Apply filters
        let files: Vec<_> = files
            .into_iter()
            .filter(|f| map.get_rank(&f.path) >= config.min_rank)
            .collect();

        let max_files = config.max_files.unwrap_or(files.len());

        for file in files.into_iter().take(max_files) {
            let file_output = Self::format_file(file, map.get_rank(&file.path), config);
            let file_tokens = Self::estimate_tokens(&file_output);

            if token_count + file_tokens > config.token_budget {
                // Try to add just the file path without symbols
                let minimal = Self::format_file_minimal(file, map.get_rank(&file.path), config);
                let minimal_tokens = Self::estimate_tokens(&minimal);

                if token_count + minimal_tokens <= config.token_budget {
                    output.push_str(&minimal);
                }
                break;
            }

            output.push_str(&file_output);
            token_count += file_tokens;
        }

        output
    }

    /// Serialize for a tool response (more detailed).
    pub fn serialize_for_tool(
        map: &RepoMap,
        focus_files: Option<&[PathBuf]>,
        query: Option<&str>,
        config: &SerializeConfig,
    ) -> String {
        let mut output = String::new();

        // If focus files are provided, start with those
        if let Some(focus) = focus_files {
            output.push_str("## Focus Files\n\n");
            for path in focus {
                if let Some(file) = map.get_file(path) {
                    let file_output =
                        Self::format_file_detailed(file, map.get_rank(&file.path), config);
                    output.push_str(&file_output);
                    output.push('\n');
                }
            }
            output.push_str("\n## Related Files\n\n");
        }

        // Filter by query if provided
        let files: Vec<_> = if let Some(q) = query {
            let q_lower = q.to_lowercase();
            map.files_by_rank()
                .into_iter()
                .filter(|f| {
                    // Match on path
                    f.path.to_string_lossy().to_lowercase().contains(&q_lower)
                        // Or match on symbol names
                        || f.symbols.iter().any(|s| s.name.to_lowercase().contains(&q_lower))
                })
                .collect()
        } else {
            map.files_by_rank()
        };

        // Skip focus files if already shown
        let focus_set: std::collections::HashSet<_> =
            focus_files.map(|f| f.iter().collect()).unwrap_or_default();

        let max_files = config.max_files.unwrap_or(50);
        let mut count = 0;

        for file in files {
            if focus_set.contains(&file.path) {
                continue;
            }

            if count >= max_files {
                break;
            }

            let file_output = Self::format_file(file, map.get_rank(&file.path), config);
            output.push_str(&file_output);
            count += 1;
        }

        output
    }

    /// Format a single file with its symbols.
    fn format_file(file: &FileInfo, rank: f64, config: &SerializeConfig) -> String {
        let mut output = String::new();

        // File header
        if config.include_ranks {
            output.push_str(&format!("{} ({:.2})\n", file.path.display(), rank));
        } else {
            output.push_str(&format!("{}\n", file.path.display()));
        }

        // Symbols (limited to most important)
        let max_symbols = 10;
        let mut symbol_count = 0;

        // Types first
        for sym in file.types().take(5) {
            if symbol_count >= max_symbols {
                break;
            }
            output.push_str(&Self::format_symbol(sym, config));
            symbol_count += 1;
        }

        // Then functions
        for sym in file.functions().take(max_symbols - symbol_count) {
            output.push_str(&Self::format_symbol(sym, config));
        }

        output
    }

    /// Format a file with just path and rank (no symbols).
    fn format_file_minimal(file: &FileInfo, rank: f64, config: &SerializeConfig) -> String {
        if config.include_ranks {
            format!("{} ({:.2})\n", file.path.display(), rank)
        } else {
            format!("{}\n", file.path.display())
        }
    }

    /// Format a file with full details (for tool output).
    fn format_file_detailed(file: &FileInfo, rank: f64, config: &SerializeConfig) -> String {
        let mut output = String::new();

        // File header
        if config.include_ranks {
            output.push_str(&format!("### {} ({:.2})\n", file.path.display(), rank));
        } else {
            output.push_str(&format!("### {}\n", file.path.display()));
        }

        // All types
        let types: Vec<_> = file.types().collect();
        if !types.is_empty() {
            output.push_str("\n**Types:**\n");
            for sym in types {
                output.push_str(&format!("- {}\n", Self::format_symbol_inline(sym)));
            }
        }

        // All functions
        let functions: Vec<_> = file.functions().collect();
        if !functions.is_empty() {
            output.push_str("\n**Functions:**\n");
            for sym in functions {
                output.push_str(&Self::format_symbol(sym, config));
            }
        }

        output
    }

    /// Format a symbol as an indented line.
    fn format_symbol(sym: &Symbol, config: &SerializeConfig) -> String {
        let prefix = Self::symbol_prefix(sym.kind);

        if config.include_signatures {
            if let Some(sig) = &sym.signature {
                // Truncate long signatures
                let sig = if sig.len() > 80 {
                    format!("{}...", &sig[..77])
                } else {
                    sig.clone()
                };
                return format!("  {} {}\n", prefix, sig);
            }
        }

        // Fall back to just name with parent if available
        if let Some(parent) = &sym.parent {
            format!("  {} {}::{}\n", prefix, parent, sym.name)
        } else {
            format!("  {} {}\n", prefix, sym.name)
        }
    }

    /// Format a symbol inline (no newline).
    fn format_symbol_inline(sym: &Symbol) -> String {
        let prefix = Self::symbol_prefix(sym.kind);
        if let Some(parent) = &sym.parent {
            format!("{} {}::{}", prefix, parent, sym.name)
        } else {
            format!("{} {}", prefix, sym.name)
        }
    }

    /// Get the prefix for a symbol kind.
    fn symbol_prefix(kind: SymbolKind) -> &'static str {
        match kind {
            SymbolKind::Function => "fn",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::TypeAlias => "type",
            SymbolKind::Constant => "const",
            SymbolKind::Module => "mod",
        }
    }

    /// Estimate token count (rough approximation: ~4 chars per token).
    fn estimate_tokens(text: &str) -> usize {
        // Simple heuristic: tokens are roughly 4 characters on average
        (text.len() + 3) / 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Language;

    fn create_test_map() -> RepoMap {
        let mut map = RepoMap::new("/project");

        let mut file1 = FileInfo::new("src/main.rs", Language::Rust);
        file1.symbols = vec![
            Symbol::new("main", SymbolKind::Function, 1)
                .with_signature("fn main()"),
            Symbol::new("Config", SymbolKind::Struct, 10),
        ];

        let mut file2 = FileInfo::new("src/lib.rs", Language::Rust);
        file2.symbols = vec![
            Symbol::new("process", SymbolKind::Function, 1)
                .with_signature("fn process(input: &str) -> Result<()>"),
            Symbol::new("Handler", SymbolKind::Trait, 20),
        ];

        map.add_file(file1);
        map.add_file(file2);
        map.ranks.insert(PathBuf::from("src/main.rs"), 0.15);
        map.ranks.insert(PathBuf::from("src/lib.rs"), 0.12);

        map
    }

    #[test]
    fn test_serialize_basic() {
        let map = create_test_map();
        let config = SerializeConfig::default();
        let output = RepoMapSerializer::serialize_for_prompt(&map, &config);

        assert!(output.contains("src/main.rs"));
        assert!(output.contains("fn main"));
        assert!(output.contains("struct Config"));
    }

    #[test]
    fn test_serialize_with_ranks() {
        let map = create_test_map();
        let config = SerializeConfig::default().ranks(true);
        let output = RepoMapSerializer::serialize_for_prompt(&map, &config);

        assert!(output.contains("(0.15)") || output.contains("(0.12)"));
    }

    #[test]
    fn test_serialize_without_ranks() {
        let map = create_test_map();
        let config = SerializeConfig::default().ranks(false);
        let output = RepoMapSerializer::serialize_for_prompt(&map, &config);

        assert!(!output.contains("(0.15)"));
        assert!(!output.contains("(0.12)"));
    }

    #[test]
    fn test_serialize_max_files() {
        let map = create_test_map();
        let config = SerializeConfig::default().max_files(1);
        let output = RepoMapSerializer::serialize_for_prompt(&map, &config);

        // Should only have one file
        let file_count = output.lines().filter(|l| !l.starts_with("  ")).count();
        assert_eq!(file_count, 1);
    }

    #[test]
    fn test_serialize_token_budget() {
        let map = create_test_map();

        // Very small budget
        let config = SerializeConfig::with_budget(10);
        let output = RepoMapSerializer::serialize_for_prompt(&map, &config);

        // Should be truncated
        let tokens = RepoMapSerializer::estimate_tokens(&output);
        assert!(tokens <= 20); // Allow some slack
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(RepoMapSerializer::estimate_tokens(""), 0);
        assert_eq!(RepoMapSerializer::estimate_tokens("test"), 1);
        assert_eq!(RepoMapSerializer::estimate_tokens("hello world"), 3); // 11 chars / 4
    }

    #[test]
    fn test_serialize_for_tool() {
        let map = create_test_map();
        let config = SerializeConfig::default();
        let output = RepoMapSerializer::serialize_for_tool(&map, None, None, &config);

        assert!(output.contains("src/main.rs"));
        assert!(output.contains("src/lib.rs"));
    }

    #[test]
    fn test_serialize_for_tool_with_query() {
        let map = create_test_map();
        let config = SerializeConfig::default();
        let output = RepoMapSerializer::serialize_for_tool(&map, None, Some("Handler"), &config);

        // Should include lib.rs which has Handler trait
        assert!(output.contains("src/lib.rs"));
    }

    #[test]
    fn test_serialize_for_tool_with_focus() {
        let map = create_test_map();
        let config = SerializeConfig::default();
        let focus = vec![PathBuf::from("src/main.rs")];
        let output = RepoMapSerializer::serialize_for_tool(&map, Some(&focus), None, &config);

        assert!(output.contains("## Focus Files"));
        assert!(output.contains("src/main.rs"));
    }
}
