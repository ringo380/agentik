//! Git operation tools.
//!
//! Provides tools for common git operations using libgit2.

use std::path::Path;

use agentik_core::tool::ToolCategory;
use agentik_core::{ToolCall, ToolDefinition, ToolResult};
use async_trait::async_trait;
use git2::{DiffOptions, Repository, Signature, StatusOptions};
use serde_json::json;

use crate::registry::{Tool, ToolContext};
use crate::ToolError;

/// Tool for showing git status.
pub struct GitStatusTool;

#[async_trait]
impl Tool for GitStatusTool {
    fn name(&self) -> &str {
        "GitStatus"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "GitStatus",
            "Shows the working tree status. Lists modified, staged, and untracked files.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {},
            "required": []
        }))
        .with_category(ToolCategory::Git)
    }

    async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let repo = open_repo(&ctx.working_dir)?;

        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false);

        let statuses = repo.statuses(Some(&mut opts))?;

        if statuses.is_empty() {
            return Ok(ToolResult::success(
                &call.id,
                "nothing to commit, working tree clean",
            ));
        }

        let mut output = String::new();
        let mut staged: Vec<String> = Vec::new();
        let mut modified: Vec<String> = Vec::new();
        let mut untracked: Vec<String> = Vec::new();

        for entry in statuses.iter() {
            let path = entry.path().unwrap_or("?");
            let status = entry.status();

            if status.is_index_new()
                || status.is_index_modified()
                || status.is_index_deleted()
                || status.is_index_renamed()
                || status.is_index_typechange()
            {
                let prefix = if status.is_index_new() {
                    "new file:   "
                } else if status.is_index_modified() {
                    "modified:   "
                } else if status.is_index_deleted() {
                    "deleted:    "
                } else if status.is_index_renamed() {
                    "renamed:    "
                } else {
                    "typechange: "
                };
                staged.push(format!("{}{}", prefix, path));
            }

            if status.is_wt_modified()
                || status.is_wt_deleted()
                || status.is_wt_renamed()
                || status.is_wt_typechange()
            {
                let prefix = if status.is_wt_modified() {
                    "modified:   "
                } else if status.is_wt_deleted() {
                    "deleted:    "
                } else if status.is_wt_renamed() {
                    "renamed:    "
                } else {
                    "typechange: "
                };
                modified.push(format!("{}{}", prefix, path));
            }

            if status.is_wt_new() {
                untracked.push(path.to_string());
            }
        }

        if !staged.is_empty() {
            output.push_str("Changes to be committed:\n");
            for item in &staged {
                output.push_str(&format!("        {}\n", item));
            }
            output.push('\n');
        }

        if !modified.is_empty() {
            output.push_str("Changes not staged for commit:\n");
            for item in &modified {
                output.push_str(&format!("        {}\n", item));
            }
            output.push('\n');
        }

        if !untracked.is_empty() {
            output.push_str("Untracked files:\n");
            for item in &untracked {
                output.push_str(&format!("        {}\n", item));
            }
        }

        Ok(ToolResult::success(&call.id, output.trim_end()))
    }
}

/// Tool for showing git diff.
pub struct GitDiffTool;

#[async_trait]
impl Tool for GitDiffTool {
    fn name(&self) -> &str {
        "GitDiff"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "GitDiff",
            "Shows changes between commits, commit and working tree, etc.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "staged": {
                    "type": "boolean",
                    "description": "Show staged changes (diff --cached). Default is false (shows unstaged changes)."
                },
                "file": {
                    "type": "string",
                    "description": "Optional file path to limit diff to."
                }
            },
            "required": []
        }))
        .with_category(ToolCategory::Git)
    }

    async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let repo = open_repo(&ctx.working_dir)?;

        let staged = call
            .arguments
            .get("staged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let file_path = call.arguments.get("file").and_then(|v| v.as_str());

        let mut opts = DiffOptions::new();
        opts.include_untracked(false);

        if let Some(path) = file_path {
            opts.pathspec(path);
        }

        let diff = if staged {
            // Staged changes: diff between HEAD and index
            let head = repo.head()?.peel_to_tree()?;
            repo.diff_tree_to_index(Some(&head), None, Some(&mut opts))?
        } else {
            // Unstaged changes: diff between index and working directory
            repo.diff_index_to_workdir(None, Some(&mut opts))?
        };

        let mut output = String::new();

        diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
            let prefix = match line.origin() {
                '+' => "+",
                '-' => "-",
                ' ' => " ",
                '=' => "=",
                '>' => ">",
                '<' => "<",
                'H' => "", // File header
                'F' => "", // File header
                'B' => "", // Binary file
                _ => "",
            };

            if let Ok(content) = std::str::from_utf8(line.content()) {
                // Skip binary file markers, handle them separately
                if line.origin() == 'B' {
                    output.push_str("Binary file differs\n");
                } else if !prefix.is_empty() || line.origin() == 'H' || line.origin() == 'F' {
                    output.push_str(prefix);
                    output.push_str(content);
                }
            }
            true
        })?;

        if output.is_empty() {
            let msg = if staged {
                "No staged changes"
            } else {
                "No unstaged changes"
            };
            Ok(ToolResult::success(&call.id, msg))
        } else {
            Ok(ToolResult::success(&call.id, output))
        }
    }
}

/// Tool for showing git log.
pub struct GitLogTool;

#[async_trait]
impl Tool for GitLogTool {
    fn name(&self) -> &str {
        "GitLog"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("GitLog", "Shows the commit log history.")
            .with_parameters(json!({
                "type": "object",
                "properties": {
                    "count": {
                        "type": "integer",
                        "description": "Number of commits to show. Default is 10."
                    },
                    "oneline": {
                        "type": "boolean",
                        "description": "Show commits in one-line format. Default is false."
                    }
                },
                "required": []
            }))
            .with_category(ToolCategory::Git)
    }

    async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let repo = open_repo(&ctx.working_dir)?;

        let count = call
            .arguments
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;

        let oneline = call
            .arguments
            .get("oneline")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut revwalk = repo.revwalk()?;
        revwalk.push_head()?;

        let mut output = String::new();

        for (shown, oid) in revwalk.enumerate() {
            if shown >= count {
                break;
            }

            let oid = oid?;
            let commit = repo.find_commit(oid)?;

            if oneline {
                let short_id = &oid.to_string()[..7];
                let message = commit.summary().unwrap_or("");
                output.push_str(&format!("{} {}\n", short_id, message));
            } else {
                output.push_str(&format!("commit {}\n", oid));

                let author_sig = commit.author();
                if let Some(author) = author_sig.name() {
                    let email = author_sig.email().unwrap_or("");
                    output.push_str(&format!("Author: {} <{}>\n", author, email));
                }

                let time = commit.time();
                let datetime = format_git_time(time.seconds());
                output.push_str(&format!("Date:   {}\n", datetime));

                output.push('\n');
                if let Some(message) = commit.message() {
                    for line in message.lines() {
                        output.push_str(&format!("    {}\n", line));
                    }
                }
                output.push('\n');
            }
        }

        if output.is_empty() {
            Ok(ToolResult::success(&call.id, "No commits found"))
        } else {
            Ok(ToolResult::success(&call.id, output.trim_end()))
        }
    }
}

/// Tool for staging files.
pub struct GitAddTool;

#[async_trait]
impl Tool for GitAddTool {
    fn name(&self) -> &str {
        "GitAdd"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("GitAdd", "Stages files for commit.")
            .with_parameters(json!({
                "type": "object",
                "properties": {
                    "files": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of file paths to stage"
                    }
                },
                "required": ["files"]
            }))
            .with_category(ToolCategory::Git)
            .destructive()
    }

    fn validate(&self, arguments: &serde_json::Value) -> Result<(), ToolError> {
        if arguments.get("files").and_then(|v| v.as_array()).is_none() {
            return Err(ToolError::missing_param("files"));
        }
        Ok(())
    }

    async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let repo = open_repo(&ctx.working_dir)?;

        let files: Vec<&str> = call.arguments["files"]
            .as_array()
            .ok_or_else(|| ToolError::missing_param("files"))?
            .iter()
            .filter_map(|v| v.as_str())
            .collect();

        if files.is_empty() {
            return Err(ToolError::invalid_args("No files specified"));
        }

        let mut index = repo.index()?;

        for file in &files {
            let path = Path::new(file);
            // Add the file to the index
            index.add_path(path)?;
        }

        index.write()?;

        Ok(ToolResult::success(
            &call.id,
            format!("Staged {} file(s)", files.len()),
        ))
    }
}

/// Tool for creating commits.
pub struct GitCommitTool;

#[async_trait]
impl Tool for GitCommitTool {
    fn name(&self) -> &str {
        "GitCommit"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("GitCommit", "Creates a new commit with staged changes.")
            .with_parameters(json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "The commit message"
                    }
                },
                "required": ["message"]
            }))
            .with_category(ToolCategory::Git)
            .requires_approval()
            .destructive()
    }

    fn validate(&self, arguments: &serde_json::Value) -> Result<(), ToolError> {
        if arguments.get("message").and_then(|v| v.as_str()).is_none() {
            return Err(ToolError::missing_param("message"));
        }
        Ok(())
    }

    async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let repo = open_repo(&ctx.working_dir)?;

        let message = call.arguments["message"]
            .as_str()
            .ok_or_else(|| ToolError::missing_param("message"))?;

        // Get the index
        let mut index = repo.index()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;

        // Get the signature (author and committer)
        let sig = repo.signature().or_else(|_| {
            // Fallback to a default signature if not configured
            Signature::now("Agentik", "agentik@example.com")
        })?;

        // Get parent commit (if any)
        let parent = match repo.head() {
            Ok(head) => Some(head.peel_to_commit()?),
            Err(_) => None, // Initial commit
        };

        let parents: Vec<&git2::Commit> = parent.iter().collect();

        // Create the commit
        let commit_id = repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)?;

        Ok(ToolResult::success(
            &call.id,
            format!(
                "Created commit {}\n\n{}",
                &commit_id.to_string()[..7],
                message
            ),
        ))
    }
}

/// Open a git repository at the given path.
fn open_repo(path: &Path) -> Result<Repository, ToolError> {
    Repository::discover(path)
        .map_err(|e| ToolError::execution(format!("Failed to open git repository: {}", e)))
}

/// Format a git timestamp as a human-readable string.
fn format_git_time(seconds: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};

    let datetime = UNIX_EPOCH + Duration::from_secs(seconds as u64);

    // Simple formatting - in production you might want chrono
    format!("{:?}", datetime)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_git_repo() -> (TempDir, Repository) {
        let dir = TempDir::new().unwrap();

        // Initialize a git repository
        let repo = Repository::init(dir.path()).unwrap();

        // Configure user for commits
        {
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "Test User").unwrap();
            config.set_str("user.email", "test@example.com").unwrap();
        }

        // Create initial file and commit
        let file_path = dir.path().join("README.md");
        fs::write(&file_path, "# Test Repo\n").unwrap();

        {
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("README.md")).unwrap();
            index.write().unwrap();

            let tree_id = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            let sig = Signature::now("Test User", "test@example.com").unwrap();

            repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
                .unwrap();
        }

        (dir, repo)
    }

    #[tokio::test]
    async fn test_git_status_clean() {
        let (dir, _repo) = setup_git_repo();
        let ctx = ToolContext::new(dir.path());
        let tool = GitStatusTool;

        let call = ToolCall::new("test", "GitStatus", json!({}));
        let result = tool.execute(&call, &ctx).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("clean"));
    }

    #[tokio::test]
    async fn test_git_status_with_changes() {
        let (dir, _repo) = setup_git_repo();

        // Modify a file
        fs::write(dir.path().join("README.md"), "# Modified\n").unwrap();

        // Create untracked file
        fs::write(dir.path().join("new_file.txt"), "new content").unwrap();

        let ctx = ToolContext::new(dir.path());
        let tool = GitStatusTool;

        let call = ToolCall::new("test", "GitStatus", json!({}));
        let result = tool.execute(&call, &ctx).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("modified"));
        assert!(result.output.contains("Untracked"));
    }

    #[tokio::test]
    async fn test_git_diff() {
        let (dir, _repo) = setup_git_repo();

        // Modify a file
        fs::write(dir.path().join("README.md"), "# Modified Content\n").unwrap();

        let ctx = ToolContext::new(dir.path());
        let tool = GitDiffTool;

        let call = ToolCall::new("test", "GitDiff", json!({}));
        let result = tool.execute(&call, &ctx).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("-# Test Repo") || result.output.contains("+# Modified"));
    }

    #[tokio::test]
    async fn test_git_log() {
        let (dir, _repo) = setup_git_repo();
        let ctx = ToolContext::new(dir.path());
        let tool = GitLogTool;

        let call = ToolCall::new("test", "GitLog", json!({ "count": 5 }));
        let result = tool.execute(&call, &ctx).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("Initial commit"));
    }

    #[tokio::test]
    async fn test_git_log_oneline() {
        let (dir, _repo) = setup_git_repo();
        let ctx = ToolContext::new(dir.path());
        let tool = GitLogTool;

        let call = ToolCall::new("test", "GitLog", json!({ "oneline": true }));
        let result = tool.execute(&call, &ctx).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("Initial commit"));
        // Oneline format should not have "Author:" etc
        assert!(!result.output.contains("Author:"));
    }

    #[tokio::test]
    async fn test_git_add() {
        let (dir, _repo) = setup_git_repo();

        // Create new file
        fs::write(dir.path().join("new_file.txt"), "new content").unwrap();

        let ctx = ToolContext::new(dir.path());
        let tool = GitAddTool;

        let call = ToolCall::new("test", "GitAdd", json!({ "files": ["new_file.txt"] }));
        let result = tool.execute(&call, &ctx).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("Staged 1 file"));
    }

    #[tokio::test]
    async fn test_git_commit() {
        let (dir, _repo) = setup_git_repo();

        // Create and stage a new file
        fs::write(dir.path().join("new_file.txt"), "new content").unwrap();
        {
            let repo = Repository::open(dir.path()).unwrap();
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("new_file.txt")).unwrap();
            index.write().unwrap();
        }

        let ctx = ToolContext::new(dir.path());
        let tool = GitCommitTool;

        let call = ToolCall::new("test", "GitCommit", json!({ "message": "Add new file" }));
        let result = tool.execute(&call, &ctx).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("Created commit"));
        assert!(result.output.contains("Add new file"));
    }
}
