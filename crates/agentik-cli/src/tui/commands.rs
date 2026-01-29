//! Slash command handling for the REPL.

use std::sync::Arc;

use agentik_agent::{Agent, AgentMode};
use agentik_session::{SessionQuery, SessionStore};

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
