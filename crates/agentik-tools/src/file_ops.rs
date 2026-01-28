//! File operation tools.
//!
//! Provides tools for reading, writing, editing, and searching files.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use agentik_core::tool::ToolCategory;
use agentik_core::{ToolCall, ToolDefinition, ToolResult};
use async_trait::async_trait;
use globset::{Glob, GlobSetBuilder};
use ignore::WalkBuilder;
use regex::Regex;
use serde_json::json;

use crate::registry::{Tool, ToolContext};
use crate::ToolError;

/// Maximum number of lines to read by default.
const DEFAULT_LINE_LIMIT: usize = 2000;

/// Maximum line length before truncation.
const MAX_LINE_LENGTH: usize = 2000;

/// Tool for reading file contents.
///
/// Reads a file and returns its contents with line numbers.
/// Supports optional offset and limit for reading portions of large files.
pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "Read"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "Read",
            "Reads a file from the filesystem. Returns contents with line numbers. \
             By default reads up to 2000 lines. Lines longer than 2000 characters are truncated.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (1-indexed). Optional."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read. Optional, defaults to 2000."
                }
            },
            "required": ["file_path"]
        }))
        .with_category(ToolCategory::FileSystem)
    }

    fn validate(&self, arguments: &serde_json::Value) -> Result<(), ToolError> {
        if arguments.get("file_path").and_then(|v| v.as_str()).is_none() {
            return Err(ToolError::missing_param("file_path"));
        }
        Ok(())
    }

    async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let file_path = call.arguments["file_path"]
            .as_str()
            .ok_or_else(|| ToolError::missing_param("file_path"))?;

        let path = ctx.resolve_path(file_path);

        // Check sandbox
        if !ctx.sandbox.is_path_allowed(&path) {
            return Err(ToolError::SandboxViolation(path));
        }

        let offset = call
            .arguments
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(1);

        let limit = call
            .arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_LINE_LIMIT);

        let content = tokio::fs::read_to_string(&path).await?;
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        // Apply offset (1-indexed) and limit
        let start = (offset.saturating_sub(1)).min(total_lines);
        let end = (start + limit).min(total_lines);

        let mut output = String::new();

        // Calculate the width needed for line numbers
        let line_num_width = end.to_string().len().max(4);

        for (idx, line) in lines[start..end].iter().enumerate() {
            let line_num = start + idx + 1;
            let truncated_line = if line.len() > MAX_LINE_LENGTH {
                format!("{}...", &line[..MAX_LINE_LENGTH])
            } else {
                line.to_string()
            };
            output.push_str(&format!(
                "{:>width$}\t{}\n",
                line_num,
                truncated_line,
                width = line_num_width
            ));
        }

        // Add info about total lines if we didn't read everything
        if end < total_lines || start > 0 {
            output.push_str(&format!(
                "\n[Showing lines {}-{} of {} total]\n",
                start + 1,
                end,
                total_lines
            ));
        }

        Ok(ToolResult::success(&call.id, output))
    }
}

/// Tool for writing file contents.
///
/// Creates a new file or overwrites an existing file with the given content.
/// Creates parent directories if they don't exist.
pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "Write"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "Write",
            "Writes content to a file. Creates the file if it doesn't exist, \
             or overwrites if it does. Creates parent directories as needed.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        }))
        .with_category(ToolCategory::FileSystem)
        .destructive()
    }

    fn validate(&self, arguments: &serde_json::Value) -> Result<(), ToolError> {
        if arguments.get("file_path").and_then(|v| v.as_str()).is_none() {
            return Err(ToolError::missing_param("file_path"));
        }
        if arguments.get("content").and_then(|v| v.as_str()).is_none() {
            return Err(ToolError::missing_param("content"));
        }
        Ok(())
    }

    async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let file_path = call.arguments["file_path"]
            .as_str()
            .ok_or_else(|| ToolError::missing_param("file_path"))?;
        let content = call.arguments["content"]
            .as_str()
            .ok_or_else(|| ToolError::missing_param("content"))?;

        let path = ctx.resolve_path(file_path);

        // Check sandbox
        if !ctx.sandbox.is_path_allowed(&path) {
            return Err(ToolError::SandboxViolation(path));
        }

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Write the file
        tokio::fs::write(&path, content).await?;

        let line_count = content.lines().count();
        let byte_count = content.len();

        Ok(ToolResult::success(
            &call.id,
            format!(
                "Successfully wrote {} lines ({} bytes) to {}",
                line_count,
                byte_count,
                path.display()
            ),
        ))
    }
}

/// Tool for editing files with search and replace.
///
/// Performs exact string replacement in a file. The old_string must be
/// unique in the file for the edit to succeed.
pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "Edit"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "Edit",
            "Performs exact string replacement in a file. The old_string must be unique \
             in the file (unless replace_all is true). Preserves file formatting.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to modify"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact text to replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The text to replace it with"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "If true, replace all occurrences. Default is false.",
                    "default": false
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        }))
        .with_category(ToolCategory::FileSystem)
        .destructive()
    }

    fn validate(&self, arguments: &serde_json::Value) -> Result<(), ToolError> {
        if arguments.get("file_path").and_then(|v| v.as_str()).is_none() {
            return Err(ToolError::missing_param("file_path"));
        }
        if arguments.get("old_string").and_then(|v| v.as_str()).is_none() {
            return Err(ToolError::missing_param("old_string"));
        }
        if arguments.get("new_string").and_then(|v| v.as_str()).is_none() {
            return Err(ToolError::missing_param("new_string"));
        }
        Ok(())
    }

    async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let file_path = call.arguments["file_path"]
            .as_str()
            .ok_or_else(|| ToolError::missing_param("file_path"))?;
        let old_string = call.arguments["old_string"]
            .as_str()
            .ok_or_else(|| ToolError::missing_param("old_string"))?;
        let new_string = call.arguments["new_string"]
            .as_str()
            .ok_or_else(|| ToolError::missing_param("new_string"))?;
        let replace_all = call
            .arguments
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let path = ctx.resolve_path(file_path);

        // Check sandbox
        if !ctx.sandbox.is_path_allowed(&path) {
            return Err(ToolError::SandboxViolation(path));
        }

        // Read current content
        let content = tokio::fs::read_to_string(&path).await?;

        // Count occurrences
        let count = content.matches(old_string).count();

        if count == 0 {
            return Err(ToolError::StringNotFound(truncate_string(old_string, 100)));
        }

        if count > 1 && !replace_all {
            return Err(ToolError::MultipleMatches(count));
        }

        // Perform replacement
        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        // Write back
        tokio::fs::write(&path, &new_content).await?;

        let msg = if replace_all && count > 1 {
            format!(
                "Replaced {} occurrences in {}",
                count,
                path.display()
            )
        } else {
            format!("Successfully edited {}", path.display())
        };

        Ok(ToolResult::success(&call.id, msg))
    }
}

/// Tool for finding files matching glob patterns.
///
/// Uses glob patterns to find files in a directory. Results are sorted
/// by modification time (most recent first).
pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "Glob",
            "Finds files matching a glob pattern. Supports patterns like '**/*.rs' or 'src/**/*.ts'. \
             Results are sorted by modification time (most recent first). Respects .gitignore.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The glob pattern to match files against"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in. Defaults to working directory."
                }
            },
            "required": ["pattern"]
        }))
        .with_category(ToolCategory::FileSystem)
    }

    fn validate(&self, arguments: &serde_json::Value) -> Result<(), ToolError> {
        if arguments.get("pattern").and_then(|v| v.as_str()).is_none() {
            return Err(ToolError::missing_param("pattern"));
        }
        Ok(())
    }

    async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let pattern = call.arguments["pattern"]
            .as_str()
            .ok_or_else(|| ToolError::missing_param("pattern"))?;

        let search_path = call
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| ctx.resolve_path(p))
            .unwrap_or_else(|| ctx.working_dir.clone());

        // Check sandbox
        if !ctx.sandbox.is_path_allowed(&search_path) {
            return Err(ToolError::SandboxViolation(search_path));
        }

        // Build glob matcher
        let glob = Glob::new(pattern)?;
        let mut builder = GlobSetBuilder::new();
        builder.add(glob);
        let glob_set = builder.build()?;

        // Walk directory and collect matching files with their mtime
        let mut matches: Vec<(std::path::PathBuf, SystemTime)> = Vec::new();

        for entry in WalkBuilder::new(&search_path)
            .hidden(false)
            .git_ignore(true)
            .build()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.is_file() {
                // Get relative path for matching
                let relative = path
                    .strip_prefix(&search_path)
                    .unwrap_or(path)
                    .to_string_lossy();

                if glob_set.is_match(&*relative) || glob_set.is_match(path) {
                    let mtime = path
                        .metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    matches.push((path.to_path_buf(), mtime));
                }
            }
        }

        // Sort by mtime (most recent first)
        matches.sort_by(|a, b| b.1.cmp(&a.1));

        // Format output
        let output = if matches.is_empty() {
            format!("No files found matching pattern: {}", pattern)
        } else {
            let file_list: Vec<String> = matches
                .iter()
                .map(|(p, _)| p.display().to_string())
                .collect();
            format!(
                "Found {} files matching '{}':\n{}",
                matches.len(),
                pattern,
                file_list.join("\n")
            )
        };

        Ok(ToolResult::success(&call.id, output))
    }
}

/// Output mode for grep results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrepOutputMode {
    /// Show matching lines with content
    Content,
    /// Show only file paths that contain matches
    FilesWithMatches,
    /// Show match counts per file
    Count,
}

impl Default for GrepOutputMode {
    fn default() -> Self {
        Self::FilesWithMatches
    }
}

impl GrepOutputMode {
    fn from_str(s: &str) -> Self {
        match s {
            "content" => Self::Content,
            "count" => Self::Count,
            _ => Self::FilesWithMatches,
        }
    }
}

/// Tool for searching file contents with regex.
///
/// Searches for patterns in files using regular expressions.
/// Supports multiple output modes and context lines.
pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "Grep",
            "Searches for a regex pattern in files. Supports context lines and multiple \
             output modes (content, files_with_matches, count). Respects .gitignore.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search in. Defaults to working directory."
                },
                "glob": {
                    "type": "string",
                    "description": "Glob pattern to filter files (e.g., '*.rs', '**/*.ts')"
                },
                "context": {
                    "type": "integer",
                    "description": "Number of context lines before and after matches"
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"],
                    "description": "Output format. 'content' shows lines, 'files_with_matches' shows paths, 'count' shows counts.",
                    "default": "files_with_matches"
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Whether to perform case-insensitive matching",
                    "default": false
                }
            },
            "required": ["pattern"]
        }))
        .with_category(ToolCategory::FileSystem)
    }

    fn validate(&self, arguments: &serde_json::Value) -> Result<(), ToolError> {
        if arguments.get("pattern").and_then(|v| v.as_str()).is_none() {
            return Err(ToolError::missing_param("pattern"));
        }
        Ok(())
    }

    async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let pattern = call.arguments["pattern"]
            .as_str()
            .ok_or_else(|| ToolError::missing_param("pattern"))?;

        let search_path = call
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| ctx.resolve_path(p))
            .unwrap_or_else(|| ctx.working_dir.clone());

        let glob_pattern = call.arguments.get("glob").and_then(|v| v.as_str());

        let context_lines = call
            .arguments
            .get("context")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(0);

        let output_mode = call
            .arguments
            .get("output_mode")
            .and_then(|v| v.as_str())
            .map(GrepOutputMode::from_str)
            .unwrap_or_default();

        let case_insensitive = call
            .arguments
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Check sandbox
        if !ctx.sandbox.is_path_allowed(&search_path) {
            return Err(ToolError::SandboxViolation(search_path));
        }

        // Build regex
        let regex_pattern = if case_insensitive {
            format!("(?i){}", pattern)
        } else {
            pattern.to_string()
        };
        let regex = Regex::new(&regex_pattern)?;

        // Build glob matcher if specified
        let glob_set = if let Some(glob_pat) = glob_pattern {
            let glob = Glob::new(glob_pat)?;
            let mut builder = GlobSetBuilder::new();
            builder.add(glob);
            Some(builder.build()?)
        } else {
            None
        };

        // Collect results
        let mut results: Vec<GrepMatch> = Vec::new();

        // Handle single file vs directory
        if search_path.is_file() {
            if let Some(matches) = search_file(&search_path, &regex, context_lines)? {
                results.push(matches);
            }
        } else {
            for entry in WalkBuilder::new(&search_path)
                .hidden(false)
                .git_ignore(true)
                .build()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }

                // Apply glob filter if specified
                if let Some(ref gs) = glob_set {
                    let relative = path
                        .strip_prefix(&search_path)
                        .unwrap_or(path)
                        .to_string_lossy();
                    if !gs.is_match(&*relative) && !gs.is_match(path) {
                        continue;
                    }
                }

                if let Some(matches) = search_file(path, &regex, context_lines)? {
                    results.push(matches);
                }
            }
        }

        // Format output based on mode
        let output = format_grep_results(&results, output_mode, pattern);

        Ok(ToolResult::success(&call.id, output))
    }
}

/// A match result from grep.
struct GrepMatch {
    path: std::path::PathBuf,
    matches: Vec<LineMatch>,
}

/// A single line match.
struct LineMatch {
    line_number: usize,
    content: String,
    context_before: Vec<String>,
    context_after: Vec<String>,
}

/// Search a file for regex matches.
fn search_file(
    path: &Path,
    regex: &Regex,
    context_lines: usize,
) -> Result<Option<GrepMatch>, ToolError> {
    // Skip binary files
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Ok(None), // Skip files we can't read as text
    };

    let lines: Vec<&str> = content.lines().collect();
    let mut matches = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        if regex.is_match(line) {
            let context_before: Vec<String> = if context_lines > 0 {
                let start = idx.saturating_sub(context_lines);
                lines[start..idx].iter().map(|s| s.to_string()).collect()
            } else {
                vec![]
            };

            let context_after: Vec<String> = if context_lines > 0 {
                let end = (idx + 1 + context_lines).min(lines.len());
                lines[idx + 1..end].iter().map(|s| s.to_string()).collect()
            } else {
                vec![]
            };

            matches.push(LineMatch {
                line_number: idx + 1,
                content: line.to_string(),
                context_before,
                context_after,
            });
        }
    }

    if matches.is_empty() {
        Ok(None)
    } else {
        Ok(Some(GrepMatch {
            path: path.to_path_buf(),
            matches,
        }))
    }
}

/// Format grep results based on output mode.
fn format_grep_results(results: &[GrepMatch], mode: GrepOutputMode, pattern: &str) -> String {
    if results.is_empty() {
        return format!("No matches found for pattern: {}", pattern);
    }

    let total_matches: usize = results.iter().map(|r| r.matches.len()).sum();

    match mode {
        GrepOutputMode::FilesWithMatches => {
            let files: Vec<String> = results.iter().map(|r| r.path.display().to_string()).collect();
            format!(
                "Found {} files with matches:\n{}",
                files.len(),
                files.join("\n")
            )
        }
        GrepOutputMode::Count => {
            let mut output = format!("Found {} total matches in {} files:\n", total_matches, results.len());
            for result in results {
                output.push_str(&format!(
                    "{}:{}\n",
                    result.path.display(),
                    result.matches.len()
                ));
            }
            output
        }
        GrepOutputMode::Content => {
            let mut output = String::new();
            for result in results {
                for m in &result.matches {
                    // Context before
                    for (i, ctx) in m.context_before.iter().enumerate() {
                        let ctx_line_num = m.line_number - m.context_before.len() + i;
                        output.push_str(&format!(
                            "{}:{}-{}\n",
                            result.path.display(),
                            ctx_line_num,
                            ctx
                        ));
                    }
                    // Matching line
                    output.push_str(&format!(
                        "{}:{}:{}\n",
                        result.path.display(),
                        m.line_number,
                        m.content
                    ));
                    // Context after
                    for (i, ctx) in m.context_after.iter().enumerate() {
                        let ctx_line_num = m.line_number + 1 + i;
                        output.push_str(&format!(
                            "{}:{}-{}\n",
                            result.path.display(),
                            ctx_line_num,
                            ctx
                        ));
                    }
                    if !m.context_before.is_empty() || !m.context_after.is_empty() {
                        output.push_str("--\n");
                    }
                }
            }
            output
        }
    }
}

/// Truncate a string for display in error messages.
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup_test_dir() -> TempDir {
        let dir = TempDir::new().unwrap();

        // Create test files
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&src_dir).unwrap();

        fs::write(
            dir.path().join("README.md"),
            "# Test Project\n\nThis is a test.\n",
        )
        .unwrap();

        fs::write(
            src_dir.join("main.rs"),
            "fn main() {\n    println!(\"Hello, world!\");\n}\n",
        )
        .unwrap();

        fs::write(
            src_dir.join("lib.rs"),
            "pub fn hello() {\n    println!(\"Hello!\");\n}\n\npub fn goodbye() {\n    println!(\"Goodbye!\");\n}\n",
        )
        .unwrap();

        dir
    }

    #[tokio::test]
    async fn test_read_tool() {
        let dir = setup_test_dir().await;
        let ctx = ToolContext::new(dir.path());
        let tool = ReadTool;

        let call = ToolCall::new(
            "test",
            "Read",
            json!({ "file_path": dir.path().join("README.md").to_string_lossy() }),
        );

        let result = tool.execute(&call, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("Test Project"));
        assert!(result.output.contains("1\t")); // Line numbers
    }

    #[tokio::test]
    async fn test_read_tool_with_offset_and_limit() {
        let dir = setup_test_dir().await;
        let ctx = ToolContext::new(dir.path());
        let tool = ReadTool;

        let call = ToolCall::new(
            "test",
            "Read",
            json!({
                "file_path": dir.path().join("src/lib.rs").to_string_lossy(),
                "offset": 2,
                "limit": 2
            }),
        );

        let result = tool.execute(&call, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("println!"));
        assert!(result.output.contains("[Showing lines 2-3"));
    }

    #[tokio::test]
    async fn test_write_tool() {
        let dir = setup_test_dir().await;
        let ctx = ToolContext::new(dir.path());
        let tool = WriteTool;

        let new_file = dir.path().join("new_file.txt");
        let call = ToolCall::new(
            "test",
            "Write",
            json!({
                "file_path": new_file.to_string_lossy(),
                "content": "Hello, World!\nLine 2\n"
            }),
        );

        let result = tool.execute(&call, &ctx).await.unwrap();
        assert!(result.success);
        assert!(new_file.exists());

        let content = fs::read_to_string(&new_file).unwrap();
        assert_eq!(content, "Hello, World!\nLine 2\n");
    }

    #[tokio::test]
    async fn test_write_tool_creates_directories() {
        let dir = setup_test_dir().await;
        let ctx = ToolContext::new(dir.path());
        let tool = WriteTool;

        let new_file = dir.path().join("deep/nested/dir/file.txt");
        let call = ToolCall::new(
            "test",
            "Write",
            json!({
                "file_path": new_file.to_string_lossy(),
                "content": "Nested content"
            }),
        );

        let result = tool.execute(&call, &ctx).await.unwrap();
        assert!(result.success);
        assert!(new_file.exists());
    }

    #[tokio::test]
    async fn test_edit_tool() {
        let dir = setup_test_dir().await;
        let ctx = ToolContext::new(dir.path());
        let tool = EditTool;

        let file = dir.path().join("src/main.rs");
        let call = ToolCall::new(
            "test",
            "Edit",
            json!({
                "file_path": file.to_string_lossy(),
                "old_string": "Hello, world!",
                "new_string": "Hello, Agentik!"
            }),
        );

        let result = tool.execute(&call, &ctx).await.unwrap();
        assert!(result.success);

        let content = fs::read_to_string(&file).unwrap();
        assert!(content.contains("Hello, Agentik!"));
        assert!(!content.contains("Hello, world!"));
    }

    #[tokio::test]
    async fn test_edit_tool_multiple_matches_error() {
        let dir = setup_test_dir().await;
        let ctx = ToolContext::new(dir.path());
        let tool = EditTool;

        let file = dir.path().join("src/lib.rs");
        let call = ToolCall::new(
            "test",
            "Edit",
            json!({
                "file_path": file.to_string_lossy(),
                "old_string": "println!",
                "new_string": "print!"
            }),
        );

        let result = tool.execute(&call, &ctx).await;
        assert!(matches!(result, Err(ToolError::MultipleMatches(2))));
    }

    #[tokio::test]
    async fn test_edit_tool_replace_all() {
        let dir = setup_test_dir().await;
        let ctx = ToolContext::new(dir.path());
        let tool = EditTool;

        let file = dir.path().join("src/lib.rs");
        let call = ToolCall::new(
            "test",
            "Edit",
            json!({
                "file_path": file.to_string_lossy(),
                "old_string": "println!",
                "new_string": "print!",
                "replace_all": true
            }),
        );

        let result = tool.execute(&call, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("2 occurrences"));

        let content = fs::read_to_string(&file).unwrap();
        assert!(!content.contains("println!"));
        assert_eq!(content.matches("print!").count(), 2);
    }

    #[tokio::test]
    async fn test_glob_tool() {
        let dir = setup_test_dir().await;
        let ctx = ToolContext::new(dir.path());
        let tool = GlobTool;

        let call = ToolCall::new(
            "test",
            "Glob",
            json!({
                "pattern": "**/*.rs",
                "path": dir.path().to_string_lossy()
            }),
        );

        let result = tool.execute(&call, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("main.rs"));
        assert!(result.output.contains("lib.rs"));
        assert!(result.output.contains("2 files"));
    }

    #[tokio::test]
    async fn test_grep_tool_files_with_matches() {
        let dir = setup_test_dir().await;
        let ctx = ToolContext::new(dir.path());
        let tool = GrepTool;

        let call = ToolCall::new(
            "test",
            "Grep",
            json!({
                "pattern": "println!",
                "path": dir.path().to_string_lossy(),
                "output_mode": "files_with_matches"
            }),
        );

        let result = tool.execute(&call, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("main.rs"));
        assert!(result.output.contains("lib.rs"));
    }

    #[tokio::test]
    async fn test_grep_tool_count() {
        let dir = setup_test_dir().await;
        let ctx = ToolContext::new(dir.path());
        let tool = GrepTool;

        let call = ToolCall::new(
            "test",
            "Grep",
            json!({
                "pattern": "println!",
                "path": dir.path().to_string_lossy(),
                "output_mode": "count"
            }),
        );

        let result = tool.execute(&call, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("3 total matches")); // 1 in main.rs + 2 in lib.rs
    }

    #[tokio::test]
    async fn test_grep_tool_content() {
        let dir = setup_test_dir().await;
        let ctx = ToolContext::new(dir.path());
        let tool = GrepTool;

        let call = ToolCall::new(
            "test",
            "Grep",
            json!({
                "pattern": "fn main",
                "path": dir.path().to_string_lossy(),
                "output_mode": "content"
            }),
        );

        let result = tool.execute(&call, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("fn main()"));
        assert!(result.output.contains(":1:")); // Line number
    }

    #[tokio::test]
    async fn test_grep_tool_with_glob() {
        let dir = setup_test_dir().await;
        let ctx = ToolContext::new(dir.path());
        let tool = GrepTool;

        let call = ToolCall::new(
            "test",
            "Grep",
            json!({
                "pattern": "println!",
                "path": dir.path().to_string_lossy(),
                "glob": "**/main.rs",
                "output_mode": "count"
            }),
        );

        let result = tool.execute(&call, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("1 total matches")); // Only main.rs
    }
}
