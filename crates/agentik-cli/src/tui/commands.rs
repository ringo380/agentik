//! Slash command handling for the REPL.

use std::path::PathBuf;
use std::sync::Arc;

use agentik_agent::{Agent, AgentMode};
use agentik_session::{SessionQuery, SessionStore};
use git2::{Repository, StatusOptions};

use crate::AppContext;

/// Result of command execution.
pub enum CommandResult {
    /// Continue the REPL loop
    Continue,
    /// Exit the REPL
    Exit,
    /// An error occurred
    Error(String),
}

/// Handle a slash command.
pub async fn handle_command(
    input: &str,
    ctx: &AppContext,
    store: &Arc<dyn SessionStore>,
    agent: &mut Agent,
) -> CommandResult {
    let parts: Vec<&str> = input.split_whitespace().collect();
    let command = parts.first().copied().unwrap_or("");
    let args = &parts[1..];

    match command {
        "/help" | "/h" | "/?" => {
            print_help();
            CommandResult::Continue
        }
        "/exit" | "/quit" | "/q" => {
            println!("Goodbye!");
            CommandResult::Exit
        }
        "/clear" => {
            // Clear screen using ANSI escape codes
            print!("\x1B[2J\x1B[1;1H");
            CommandResult::Continue
        }
        "/session" => handle_session_command(args, store, agent.session().id()).await,
        "/model" => handle_model_command(args, ctx),
        "/provider" => handle_provider_command(args, ctx),
        "/status" => print_status(ctx, store, agent).await,
        "/history" => handle_history_command(args, store, agent.session().id()).await,
        "/mode" => handle_mode_command(args, agent),
        "/tools" => handle_tools_command(args, agent),
        "/compact" => handle_compact_command(agent).await,
        "/add" => handle_add_command(args, store, agent).await,
        "/drop" => handle_drop_command(args, store, agent).await,
        "/files" => handle_files_command(agent),
        "/undo" => handle_undo_command(agent),
        _ => CommandResult::Error(format!(
            "Unknown command: {}. Type /help for available commands.",
            command
        )),
    }
}

/// Print help information.
fn print_help() {
    println!("Available commands:");
    println!();
    println!("  /help, /h, /?    Show this help message");
    println!("  /exit, /quit, /q Exit the REPL");
    println!("  /clear           Clear the screen");
    println!("  /session         Show current session info");
    println!("  /session list    List recent sessions");
    println!("  /session title [text]      Show or set session title");
    println!("  /session tag               List session tags");
    println!("  /session tag add <tag>     Add a tag");
    println!("  /session tag remove <tag>  Remove a tag");
    println!("  /model           Show current model");
    println!("  /model <name>    Switch to a different model");
    println!("  /provider        Show current provider");
    println!("  /provider list   List available providers");
    println!("  /status          Show status information");
    println!("  /history         Show conversation history");
    println!();
    println!("Agent commands:");
    println!("  /mode            Show current agent mode");
    println!("  /mode <mode>     Switch mode (supervised, autonomous, planning, ask)");
    println!("  /tools           List available tools");
    println!("  /compact         Trigger context compaction");
    println!();
    println!("Context commands:");
    println!("  /add <path>      Add file or directory to context");
    println!("  /drop <path>     Remove file from context");
    println!("  /files           List files in context");
    println!();
    println!("Git commands:");
    println!("  /undo            Undo the last git commit");
    println!();
    println!("Tool Approval (when prompted):");
    println!("  y, yes           Approve this tool call");
    println!("  n, no            Deny this tool call");
    println!("  a, always        Approve and auto-approve future calls to this tool");
    println!("  q, quit          Deny and stop the current operation");
    println!();
    println!("Tips:");
    println!("  - Press Ctrl+D to exit");
    println!("  - Press Ctrl+C to cancel current operation");
    println!("  - Use Up/Down arrows for command history");
    println!("  - Start agentik with -c to continue your last session");
    println!("  - Start agentik with -r <id> to resume a specific session");
    println!("  - Start agentik with --plan for planning mode");
}

/// Handle /session command.
async fn handle_session_command(
    args: &[&str],
    store: &Arc<dyn SessionStore>,
    session_id: &str,
) -> CommandResult {
    match args.first() {
        Some(&"list") => {
            let limit = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
            match store.list(&SessionQuery::new().with_limit(limit)).await {
                Ok(sessions) => {
                    println!("Recent sessions:");
                    println!();
                    for s in sessions {
                        let marker = if s.id == session_id { " *" } else { "  " };
                        let title = s.title.as_deref().unwrap_or("(untitled)");
                        println!(
                            "{}  {} | {} | {} messages | {}",
                            marker,
                            &s.id[..8],
                            title,
                            s.message_count,
                            s.last_active_at.format("%Y-%m-%d %H:%M")
                        );
                    }
                    CommandResult::Continue
                }
                Err(e) => CommandResult::Error(format!("Failed to list sessions: {}", e)),
            }
        }
        Some(&"title") => handle_session_title(&args[1..], store, session_id).await,
        Some(&"tag") => handle_session_tag(&args[1..], store, session_id).await,
        None => match store.get_metadata(session_id).await {
            Ok(meta) => {
                println!("Current session:");
                println!("  ID:         {}", meta.id);
                println!(
                    "  Title:      {}",
                    meta.title.as_deref().unwrap_or("(untitled)")
                );
                println!(
                    "  Created:    {}",
                    meta.created_at.format("%Y-%m-%d %H:%M:%S")
                );
                println!(
                    "  Last active: {}",
                    meta.last_active_at.format("%Y-%m-%d %H:%M:%S")
                );
                println!("  Directory:  {}", meta.working_directory.display());
                if !meta.tags.is_empty() {
                    println!("  Tags:       {}", meta.tags.join(", "));
                }
                CommandResult::Continue
            }
            Err(e) => CommandResult::Error(format!("Failed to get session info: {}", e)),
        },
        Some(subcmd) => CommandResult::Error(format!(
            "Unknown session subcommand: {}. Try /session, /session list, /session title, or /session tag",
            subcmd
        )),
    }
}

/// Handle /session title command.
async fn handle_session_title(
    args: &[&str],
    store: &Arc<dyn SessionStore>,
    session_id: &str,
) -> CommandResult {
    match store.get_metadata(session_id).await {
        Ok(mut meta) => {
            if args.is_empty() {
                // Show current title
                println!(
                    "Current title: {}",
                    meta.title.as_deref().unwrap_or("(untitled)")
                );
            } else {
                // Set new title
                let new_title = args.join(" ");
                if new_title.len() > 100 {
                    return CommandResult::Error(
                        "Title too long (max 100 characters)".to_string(),
                    );
                }
                meta.title = Some(new_title.clone());
                match store.update_metadata(&meta).await {
                    Ok(()) => println!("Title set to: {}", new_title),
                    Err(e) => return CommandResult::Error(format!("Failed to update title: {}", e)),
                }
            }
            CommandResult::Continue
        }
        Err(e) => CommandResult::Error(format!("Failed to get session info: {}", e)),
    }
}

/// Handle /session tag command.
async fn handle_session_tag(
    args: &[&str],
    store: &Arc<dyn SessionStore>,
    session_id: &str,
) -> CommandResult {
    match store.get_metadata(session_id).await {
        Ok(mut meta) => {
            match args.first() {
                Some(&"add") => {
                    if args.len() < 2 {
                        return CommandResult::Error(
                            "Usage: /session tag add <tag>".to_string(),
                        );
                    }
                    let tag = args[1..].join(" ");
                    if tag.len() > 50 {
                        return CommandResult::Error(
                            "Tag too long (max 50 characters)".to_string(),
                        );
                    }
                    if tag.contains(',') {
                        return CommandResult::Error(
                            "Tags cannot contain commas".to_string(),
                        );
                    }
                    if meta.tags.contains(&tag) {
                        println!("Tag already exists: {}", tag);
                    } else {
                        meta.tags.push(tag.clone());
                        match store.update_metadata(&meta).await {
                            Ok(()) => println!("Tag added: {}", tag),
                            Err(e) => {
                                return CommandResult::Error(format!("Failed to add tag: {}", e))
                            }
                        }
                    }
                }
                Some(&"remove") => {
                    if args.len() < 2 {
                        return CommandResult::Error(
                            "Usage: /session tag remove <tag>".to_string(),
                        );
                    }
                    let tag = args[1..].join(" ");
                    if let Some(pos) = meta.tags.iter().position(|t| t == &tag) {
                        meta.tags.remove(pos);
                        match store.update_metadata(&meta).await {
                            Ok(()) => println!("Tag removed: {}", tag),
                            Err(e) => {
                                return CommandResult::Error(format!("Failed to remove tag: {}", e))
                            }
                        }
                    } else {
                        println!("Tag not found: {}", tag);
                    }
                }
                Some(&"list") | None => {
                    if meta.tags.is_empty() {
                        println!("No tags set");
                    } else {
                        println!("Tags: {}", meta.tags.join(", "));
                    }
                }
                Some(subcmd) => {
                    return CommandResult::Error(format!(
                        "Unknown tag subcommand: {}. Try add, remove, or list",
                        subcmd
                    ));
                }
            }
            CommandResult::Continue
        }
        Err(e) => CommandResult::Error(format!("Failed to get session info: {}", e)),
    }
}

/// Handle /model command.
fn handle_model_command(args: &[&str], ctx: &AppContext) -> CommandResult {
    if args.is_empty() {
        // Show current model and available models
        println!("Current model: {}", ctx.config.general.model);
        println!();
        println!("Available models:");
        if let Some(provider) = ctx.registry.default_provider() {
            for model in provider.available_models() {
                let marker = if model.id == ctx.config.general.model {
                    " *"
                } else {
                    "  "
                };
                println!(
                    "{}  {} ({}) - {}k context",
                    marker,
                    model.id,
                    model.name,
                    model.context_window / 1000
                );
            }
        }
        CommandResult::Continue
    } else {
        let model_name = args[0];
        // TODO: Actually switch the model (requires mutable config)
        println!(
            "Model switching not yet implemented. Current model: {}",
            ctx.config.general.model
        );
        println!(
            "To use a different model, restart with: agentik -m {}",
            model_name
        );
        CommandResult::Continue
    }
}

/// Handle /provider command.
fn handle_provider_command(args: &[&str], ctx: &AppContext) -> CommandResult {
    match args.first() {
        Some(&"list") | None => {
            println!("Available providers:");
            for provider in ctx.registry.providers() {
                let status = if provider.is_configured() {
                    "configured"
                } else {
                    "not configured"
                };
                let is_default = ctx
                    .registry
                    .default_provider()
                    .map(|p| p.id() == provider.id())
                    .unwrap_or(false);
                let marker = if is_default { " *" } else { "  " };
                println!(
                    "{}  {} ({}) - {}",
                    marker,
                    provider.id(),
                    provider.name(),
                    status
                );
            }
            CommandResult::Continue
        }
        Some(subcmd) => CommandResult::Error(format!("Unknown provider subcommand: {}", subcmd)),
    }
}

/// Print status information.
async fn print_status(
    ctx: &AppContext,
    store: &Arc<dyn SessionStore>,
    agent: &Agent,
) -> CommandResult {
    println!("Agentik Status");
    println!("==============");
    println!();

    // Provider info
    if let Some(provider) = ctx.registry.default_provider() {
        println!("Provider: {} ({})", provider.name(), provider.id());
        println!("Model:    {}", agent.config().model);
    } else {
        println!("Provider: Not configured");
    }
    println!();

    // Agent info
    println!("Mode:     {:?}", agent.mode());
    println!();

    // Session info
    let session_id = agent.session().id();
    println!("Session:  {}", session_id);
    if let Ok(meta) = store.get_metadata(session_id).await {
        println!("Turns:    {} in session", meta.metrics.turn_count);
        println!(
            "Tokens:   {} in / {} out",
            meta.metrics.total_tokens_in, meta.metrics.total_tokens_out
        );
    }
    println!();

    // Context manager info
    let context = agent.context_manager();
    let usage = context.calculate_usage(agent.session());
    println!("Context tokens: {} ({:.1}% of max)", usage.total_tokens, usage.usage_percent * 100.0);
    println!();

    // Config info
    println!("Max tokens:     {}", ctx.config.limits.max_tokens);
    println!(
        "Sandbox:        {}",
        if ctx.config.general.sandbox {
            "enabled"
        } else {
            "disabled"
        }
    );

    CommandResult::Continue
}

/// Handle /history command.
async fn handle_history_command(
    args: &[&str],
    store: &Arc<dyn SessionStore>,
    session_id: &str,
) -> CommandResult {
    let limit = args.first().and_then(|s| s.parse().ok()).unwrap_or(10);

    match store.get_messages(session_id, None, Some(limit)).await {
        Ok(messages) => {
            if messages.is_empty() {
                println!("No messages in this session yet.");
            } else {
                println!("Recent messages (showing up to {}):", limit);
                println!();
                for msg in messages {
                    let role = format!("{:?}", msg.role).to_uppercase();
                    let content = msg.content.as_text();
                    let preview = if content.len() > 80 {
                        format!("{}...", &content[..80])
                    } else {
                        content.to_string()
                    };
                    println!("[{}] {}", role, preview);
                }
            }
            CommandResult::Continue
        }
        Err(e) => CommandResult::Error(format!("Failed to get history: {}", e)),
    }
}

/// Handle /mode command.
fn handle_mode_command(args: &[&str], agent: &mut Agent) -> CommandResult {
    if args.is_empty() {
        // Show current mode and available modes
        println!("Current mode: {:?}", agent.mode());
        println!();
        println!("Available modes:");
        println!("  supervised   Ask before each tool execution (default)");
        println!("  autonomous   Execute tools without asking");
        println!("  planning     Create plans but don't execute");
        println!("  architect    High-level design without implementation");
        println!("  ask          Answer questions only, no tool use");
        CommandResult::Continue
    } else {
        let mode = match args[0].to_lowercase().as_str() {
            "supervised" => AgentMode::Supervised,
            "autonomous" => AgentMode::Autonomous,
            "planning" => AgentMode::Planning,
            "architect" => AgentMode::Architect,
            "ask" | "askonly" | "ask-only" => AgentMode::AskOnly,
            other => {
                return CommandResult::Error(format!(
                    "Unknown mode: '{}'. Available: supervised, autonomous, planning, architect, ask",
                    other
                ));
            }
        };

        agent.set_mode(mode);
        println!("[Mode changed to {:?}]", mode);
        CommandResult::Continue
    }
}

/// Handle /tools command.
fn handle_tools_command(args: &[&str], _agent: &Agent) -> CommandResult {
    // Get tools from the agent's context (through executor)
    // Since we can't directly access the executor, we'll show a message
    // In a full implementation, we'd expose the tool list through the Agent API

    if args.first() == Some(&"list") || args.is_empty() {
        println!("Available tools:");
        println!();

        // The executor is private, but we can at least show built-in tool names
        println!("Built-in tools:");
        println!("  Read        Read file contents");
        println!("  Write       Write content to a file");
        println!("  Edit        Edit file with search/replace");
        println!("  Glob        Find files matching a pattern");
        println!("  Grep        Search file contents");
        println!("  Bash        Execute shell commands");
        println!("  ListDir     List directory contents");
        println!();
        println!("Use /mode to change how tools are approved:");
        println!("  supervised  - Ask before each tool (current default)");
        println!("  autonomous  - Execute without asking");

        CommandResult::Continue
    } else {
        CommandResult::Error("Usage: /tools or /tools list".to_string())
    }
}

/// Handle /compact command.
async fn handle_compact_command(agent: &mut Agent) -> CommandResult {
    println!("[Triggering context compaction...]");

    match agent.compact().await {
        Ok(()) => {
            let usage = agent.context_manager().calculate_usage(agent.session());
            println!("[Context compaction complete]");
            println!("Current context tokens: {} ({:.1}% of max)", usage.total_tokens, usage.usage_percent * 100.0);
            CommandResult::Continue
        }
        Err(e) => CommandResult::Error(format!("Compaction failed: {}", e)),
    }
}

/// Handle /add command to add files to context.
async fn handle_add_command(
    args: &[&str],
    store: &Arc<dyn SessionStore>,
    agent: &mut Agent,
) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /add <path> [path...]".to_string());
    }

    let working_dir = agent.session().metadata.working_directory.clone();
    let mut added = Vec::new();
    let mut errors = Vec::new();

    for arg in args {
        let pattern = working_dir.join(arg);
        let pattern_str = pattern.to_string_lossy();

        // Try glob expansion
        match glob::glob(&pattern_str) {
            Ok(paths) => {
                let mut matched_any = false;
                for entry in paths.flatten() {
                    matched_any = true;
                    // Make path relative to working directory
                    let relative_path = entry
                        .strip_prefix(&working_dir)
                        .unwrap_or(&entry)
                        .to_path_buf();

                    if entry.is_file() {
                        add_file_to_context(agent, relative_path.clone(), &mut added);
                    } else if entry.is_dir() {
                        add_directory_to_context(agent, &entry, &working_dir, &mut added);
                    }
                }

                // If glob didn't match, try as literal path
                if !matched_any {
                    let path = PathBuf::from(arg);
                    let full_path = working_dir.join(&path);

                    if full_path.is_file() {
                        add_file_to_context(agent, path, &mut added);
                    } else if full_path.is_dir() {
                        add_directory_to_context(agent, &full_path, &working_dir, &mut added);
                    } else {
                        errors.push(format!("Not found: {}", arg));
                    }
                }
            }
            Err(e) => {
                errors.push(format!("Invalid pattern '{}': {}", arg, e));
            }
        }
    }

    // Update metadata and persist
    if !added.is_empty() {
        if let Err(e) = store.update_metadata(&agent.session().metadata).await {
            return CommandResult::Error(format!("Failed to persist: {}", e));
        }
    }

    // Print results
    for path in &added {
        println!("Added: {}", path.display());
    }
    for err in &errors {
        println!("{}", err);
    }

    if added.is_empty() && errors.is_empty() {
        println!("No files added.");
    }

    CommandResult::Continue
}

/// Add a single file to context.
fn add_file_to_context(agent: &mut Agent, path: PathBuf, added: &mut Vec<PathBuf>) {
    let files = &mut agent.session_mut().metadata.added_files;
    if !files.contains(&path) {
        files.push(path.clone());
        added.push(path);
    }
}

/// Recursively add directory contents to context.
fn add_directory_to_context(
    agent: &mut Agent,
    dir: &std::path::Path,
    working_dir: &std::path::Path,
    added: &mut Vec<PathBuf>,
) {
    const MAX_DIR_FILES: usize = 50;
    let mut count = 0;

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if count >= MAX_DIR_FILES {
                println!("  (limited to {} files)", MAX_DIR_FILES);
                break;
            }

            let path = entry.path();
            if path.is_file() {
                let relative = path
                    .strip_prefix(working_dir)
                    .unwrap_or(&path)
                    .to_path_buf();
                add_file_to_context(agent, relative, added);
                count += 1;
            }
        }
    }
}

/// Handle /drop command to remove files from context.
async fn handle_drop_command(
    args: &[&str],
    store: &Arc<dyn SessionStore>,
    agent: &mut Agent,
) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error("Usage: /drop <path> [path...]".to_string());
    }

    let working_dir = agent.session().metadata.working_directory.clone();
    let mut removed = Vec::new();

    for arg in args {
        let pattern = working_dir.join(arg);
        let pattern_str = pattern.to_string_lossy();

        // Get current files to match against
        let current_files: Vec<PathBuf> = agent
            .session()
            .metadata
            .added_files
            .clone();

        // Try glob pattern matching
        let matched: Vec<PathBuf> = current_files
            .iter()
            .filter(|f| {
                let full_path = working_dir.join(f);
                let full_str = full_path.to_string_lossy();
                glob::Pattern::new(&pattern_str)
                    .map(|p| p.matches(&full_str))
                    .unwrap_or(false)
                    || f.to_string_lossy() == *arg
                    || f.starts_with(arg)
            })
            .cloned()
            .collect();

        if matched.is_empty() {
            // Try exact match
            let path = PathBuf::from(arg);
            let files = &mut agent.session_mut().metadata.added_files;
            if let Some(pos) = files.iter().position(|f| f == &path) {
                let removed_path = files.remove(pos);
                removed.push(removed_path);
            }
        } else {
            for path in matched {
                let files = &mut agent.session_mut().metadata.added_files;
                if let Some(pos) = files.iter().position(|f| f == &path) {
                    files.remove(pos);
                    removed.push(path);
                }
            }
        }
    }

    // Update metadata and persist
    if !removed.is_empty() {
        if let Err(e) = store.update_metadata(&agent.session().metadata).await {
            return CommandResult::Error(format!("Failed to persist: {}", e));
        }
    }

    // Print results
    if removed.is_empty() {
        println!("No matching files found in context.");
    } else {
        for path in &removed {
            println!("Removed: {}", path.display());
        }
    }

    CommandResult::Continue
}

/// Handle /files command to list files in context.
fn handle_files_command(agent: &Agent) -> CommandResult {
    let files = &agent.session().metadata.added_files;

    if files.is_empty() {
        println!("No files in context.");
        println!();
        println!("Use /add <path> to add files.");
        return CommandResult::Continue;
    }

    println!("Files in context:");
    println!();

    let working_dir = &agent.session().metadata.working_directory;
    let mut total_size = 0usize;

    for path in files {
        let full_path = working_dir.join(path);
        let size_info = if let Ok(metadata) = std::fs::metadata(&full_path) {
            let size = metadata.len() as usize;
            total_size += size;
            format_size(size)
        } else {
            "(not found)".to_string()
        };
        println!("  {} ({})", path.display(), size_info);
    }

    println!();
    println!(
        "Total: {} files, {} (~{} tokens)",
        files.len(),
        format_size(total_size),
        total_size / 4 // rough token estimate
    );

    CommandResult::Continue
}

/// Format a size in human-readable form.
fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Handle /undo command to revert the last git commit.
fn handle_undo_command(agent: &Agent) -> CommandResult {
    let working_dir = &agent.session().metadata.working_directory;

    // Open the repository
    let repo = match Repository::discover(working_dir) {
        Ok(r) => r,
        Err(_) => return CommandResult::Error("Not in a git repository".to_string()),
    };

    // Check for uncommitted changes
    if has_uncommitted_changes(&repo) {
        return CommandResult::Error(
            "Cannot undo: you have uncommitted changes. Commit or stash them first.".to_string(),
        );
    }

    // Get the HEAD commit
    let head = match repo.head() {
        Ok(h) => h,
        Err(e) => return CommandResult::Error(format!("Failed to get HEAD: {}", e)),
    };

    let head_commit = match head.peel_to_commit() {
        Ok(c) => c,
        Err(e) => return CommandResult::Error(format!("Failed to get HEAD commit: {}", e)),
    };

    // Get commit info for display
    let message = head_commit.summary().unwrap_or("(no message)");
    let short_sha = &head_commit.id().to_string()[..7];

    // Find the parent commit
    let parent = match head_commit.parent(0) {
        Ok(p) => p,
        Err(_) => {
            return CommandResult::Error("Cannot undo: this is the initial commit.".to_string())
        }
    };

    // Perform hard reset to the parent
    if let Err(e) = repo.reset(parent.as_object(), git2::ResetType::Hard, None) {
        return CommandResult::Error(format!("Failed to undo: {}", e));
    }

    println!("Undid commit {} ({})", short_sha, message);
    CommandResult::Continue
}

/// Check if the repository has uncommitted changes.
fn has_uncommitted_changes(repo: &Repository) -> bool {
    let mut opts = StatusOptions::new();
    opts.include_untracked(false);

    match repo.statuses(Some(&mut opts)) {
        Ok(statuses) => !statuses.is_empty(),
        Err(_) => true, // Assume dirty on error
    }
}
