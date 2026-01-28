//! Slash command handling for the REPL.

use agentik_session::{SessionQuery, SessionStore, SqliteSessionStore};

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
    store: &SqliteSessionStore,
    session_id: &str,
) -> CommandResult {
    let parts: Vec<&str> = input.split_whitespace().collect();
    let command = parts.first().map(|s| *s).unwrap_or("");
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
        "/session" => handle_session_command(args, store, session_id).await,
        "/model" => handle_model_command(args, ctx),
        "/provider" => handle_provider_command(args, ctx),
        "/status" => print_status(ctx, store, session_id).await,
        "/history" => handle_history_command(args, store, session_id).await,
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
    println!("  /model           Show current model");
    println!("  /model <name>    Switch to a different model");
    println!("  /provider        Show current provider");
    println!("  /provider list   List available providers");
    println!("  /status          Show status information");
    println!("  /history         Show conversation history");
    println!();
    println!("Tips:");
    println!("  - Press Ctrl+D to exit");
    println!("  - Use Up/Down arrows for command history");
    println!("  - Start agentik with -c to continue your last session");
    println!("  - Start agentik with -r <id> to resume a specific session");
}

/// Handle /session command.
async fn handle_session_command(
    args: &[&str],
    store: &SqliteSessionStore,
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
            "Unknown session subcommand: {}. Try /session or /session list",
            subcmd
        )),
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
    store: &SqliteSessionStore,
    session_id: &str,
) -> CommandResult {
    println!("Agentik Status");
    println!("==============");
    println!();

    // Provider info
    if let Some(provider) = ctx.registry.default_provider() {
        println!("Provider: {} ({})", provider.name(), provider.id());
        println!("Model:    {}", ctx.config.general.model);
    } else {
        println!("Provider: Not configured");
    }
    println!();

    // Session info
    println!("Session:  {}", session_id);
    if let Ok(meta) = store.get_metadata(session_id).await {
        println!("Turns:    {} in session", meta.metrics.turn_count);
        println!(
            "Tokens:   {} in / {} out",
            meta.metrics.total_tokens_in, meta.metrics.total_tokens_out
        );
    }
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
    store: &SqliteSessionStore,
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
