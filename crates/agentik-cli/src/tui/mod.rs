//! Terminal UI for interactive mode.
//!
//! Provides a readline-style REPL with:
//! - Input history
//! - Slash commands
//! - Streaming response display
//! - Session management
//! - Tool execution with approval workflows

use std::path::PathBuf;
use std::sync::Arc;

use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use agentik_agent::{Agent, AgentBuilder, AgentMode, ExecutorBuilder};
use agentik_core::Session;
use agentik_session::{SessionStore, SqliteSessionStore};

use crate::{AppContext, Cli};

mod commands;
mod handlers;

pub use handlers::{CliEventHandler, CliPermissionHandler};

/// Run the interactive REPL.
pub async fn run(cli: Cli, ctx: Arc<AppContext>) -> anyhow::Result<()> {
    // Initialize session store
    let store = SqliteSessionStore::open_default()?;
    let store = Arc::new(store) as Arc<dyn SessionStore>;

    // Create or resume session
    let session = create_or_resume_session(&cli, &store).await?;
    let session_id = session.id().to_string();

    // Print welcome banner
    print_welcome_banner(&cli, &ctx);

    // Show session info
    println!(
        "[Session: {} | Messages: {}]",
        &session_id[..8],
        session.messages.len()
    );
    println!();

    // Create the agent
    let mut agent = create_agent(&cli, &ctx, store.clone(), session)?;

    // Show initial mode
    println!("[Mode: {:?}]", agent.mode());
    println!();

    // Initialize readline editor
    let mut editor = DefaultEditor::new()?;

    // Load history if it exists
    let history_path = get_history_path();
    if history_path.exists() {
        let _ = editor.load_history(&history_path);
    }

    // Main REPL loop
    loop {
        let prompt = format!(
            "{}>>> ",
            if agent.session().messages.is_empty() {
                ""
            } else {
                "\n"
            }
        );

        match editor.readline(&prompt) {
            Ok(line) => {
                let line = line.trim();

                if line.is_empty() {
                    continue;
                }

                // Add to history
                let _ = editor.add_history_entry(line);

                // Handle slash commands
                if line.starts_with('/') {
                    match commands::handle_command(line, &ctx, &store, &mut agent).await {
                        commands::CommandResult::Continue => continue,
                        commands::CommandResult::Exit => break,
                        commands::CommandResult::Error(e) => {
                            eprintln!("Error: {}", e);
                            continue;
                        }
                    }
                }

                // Process as a message to the AI
                if let Err(e) = process_message(line, &mut agent).await {
                    eprintln!("Error: {}", e);
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                // Cancel any ongoing operation
                agent.cancel();
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("\nGoodbye!");
                break;
            }
            Err(err) => {
                eprintln!("Error: {:?}", err);
                break;
            }
        }
    }

    // Save history
    if let Some(parent) = history_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = editor.save_history(&history_path);

    Ok(())
}

/// Create the agent with CLI handlers and tools.
fn create_agent(
    cli: &Cli,
    ctx: &AppContext,
    store: Arc<dyn SessionStore>,
    session: Session,
) -> anyhow::Result<Agent> {
    // Get the provider
    let provider = ctx.registry.default_provider().ok_or_else(|| {
        anyhow::anyhow!("No provider configured. Set ANTHROPIC_API_KEY or OPENAI_API_KEY.")
    })?;

    // Create handlers
    let event_handler = Arc::new(CliEventHandler::new());
    let permission_handler = Arc::new(CliPermissionHandler::new());

    // Determine initial mode
    let mode = if cli.plan {
        AgentMode::Planning
    } else {
        AgentMode::Supervised
    };

    // Build tool executor with builtins
    let working_dir = session.metadata.working_directory.clone();
    let executor = ExecutorBuilder::new()
        .with_builtins()
        .working_dir(&working_dir)
        .permissions(ctx.config.permissions.clone())
        .mode(mode)
        .build(permission_handler);

    // Determine model
    let model = cli
        .model
        .clone()
        .unwrap_or_else(|| ctx.config.general.model.clone());

    // Build the agent
    let agent = AgentBuilder::new()
        .provider(provider)
        .executor(executor)
        .store(store)
        .session(session)
        .model(model)
        .max_tokens(ctx.config.limits.max_tokens)
        .temperature(0.7)
        .event_handler(event_handler)
        .mode(mode)
        .build()?;

    Ok(agent)
}

/// Create a new session or resume an existing one based on CLI flags.
async fn create_or_resume_session(
    cli: &Cli,
    store: &Arc<dyn SessionStore>,
) -> anyhow::Result<Session> {
    // Try to resume specific session
    if let Some(ref session_id) = cli.resume {
        // Try exact match first
        match store.get(session_id).await {
            Ok(session) => {
                println!("[Resuming session {}]", session_id);
                return Ok(session);
            }
            Err(_) => {
                // Try prefix match
                let matches = store.find_by_prefix(session_id).await?;
                if matches.len() == 1 {
                    let session = store.get(&matches[0].id).await?;
                    println!("[Resuming session {}]", matches[0].id);
                    return Ok(session);
                } else if matches.len() > 1 {
                    anyhow::bail!(
                        "Ambiguous session ID '{}'. Matches: {}",
                        session_id,
                        matches
                            .iter()
                            .map(|s| &s.id[..8])
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                } else {
                    anyhow::bail!("Session '{}' not found", session_id);
                }
            }
        }
    }

    // Try to continue most recent session
    if cli.r#continue {
        if let Some(summary) = store.get_most_recent().await? {
            let session = store.get(&summary.id).await?;
            println!("[Continuing session {}]", &summary.id[..8]);
            return Ok(session);
        } else {
            println!("[No previous session found, starting new session]");
        }
    }

    // Create new session
    let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let session = Session::new(working_dir);
    store.create(&session).await?;

    Ok(session)
}

/// Process a user message using the agent.
async fn process_message(input: &str, agent: &mut Agent) -> anyhow::Result<()> {
    // Reset cancellation for new message
    agent.reset_cancel();

    // Run the agent - event handler streams text via on_text_delta
    println!();
    match agent.run(input).await {
        Ok(_response) => {
            // Response is already streamed via event handler
            Ok(())
        }
        Err(agentik_agent::AgentError::Cancelled) => {
            eprintln!("\n[Operation cancelled]");
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

/// Print the welcome banner.
fn print_welcome_banner(cli: &Cli, ctx: &AppContext) {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!(
        "║  agentik v{}                                              ║",
        env!("CARGO_PKG_VERSION")
    );
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  Type /help for commands, or start chatting.                 ║");
    println!("║  Press Ctrl+D to exit, Ctrl+C to cancel.                     ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // Show provider info
    if let Some(provider) = ctx.registry.default_provider() {
        let model = cli.model.as_deref().unwrap_or(&ctx.config.general.model);
        println!("[Provider: {} | Model: {}]", provider.name(), model);
    } else {
        println!("[Warning: No provider configured. Set ANTHROPIC_API_KEY or OPENAI_API_KEY]");
    }

    if cli.plan {
        println!("[Planning mode enabled]");
    }

    if !cli.add.is_empty() {
        println!("[Added to context: {:?}]", cli.add);
    }

    if let Some(ref prompt) = cli.prompt {
        println!("\n[Initial prompt: {}]", prompt);
    }
}

/// Get the path to the history file.
fn get_history_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("agentik")
        .join("history.txt")
}
