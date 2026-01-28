//! Terminal UI for interactive mode.
//!
//! Provides a readline-style REPL with:
//! - Input history
//! - Slash commands
//! - Streaming response display
//! - Session management

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use agentik_core::{Message, Session};
use agentik_providers::CompletionRequest;
use agentik_session::{SessionStore, SqliteSessionStore};

use crate::{AppContext, Cli};

mod commands;

/// Run the interactive REPL.
pub async fn run(cli: Cli, ctx: Arc<AppContext>) -> anyhow::Result<()> {
    // Initialize session store
    let store = SqliteSessionStore::open_default()?;

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

    // Initialize readline editor
    let mut editor = DefaultEditor::new()?;

    // Load history if it exists
    let history_path = get_history_path();
    if history_path.exists() {
        let _ = editor.load_history(&history_path);
    }

    // Main REPL loop
    let mut messages: Vec<Message> = session.messages.clone();

    loop {
        let prompt = format!("{}>>> ", if messages.is_empty() { "" } else { "\n" });

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
                    match commands::handle_command(line, &ctx, &store, &session_id).await {
                        commands::CommandResult::Continue => continue,
                        commands::CommandResult::Exit => break,
                        commands::CommandResult::Error(e) => {
                            eprintln!("Error: {}", e);
                            continue;
                        }
                    }
                }

                // Process as a message to the AI
                if let Err(e) =
                    process_message(line, &mut messages, &ctx, &store, &session_id).await
                {
                    eprintln!("Error: {}", e);
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                // Don't exit on Ctrl-C, just cancel current input
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

/// Create a new session or resume an existing one based on CLI flags.
async fn create_or_resume_session(
    cli: &Cli,
    store: &SqliteSessionStore,
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

/// Process a user message and get a response from the AI.
async fn process_message(
    input: &str,
    messages: &mut Vec<Message>,
    ctx: &AppContext,
    store: &SqliteSessionStore,
    session_id: &str,
) -> anyhow::Result<()> {
    // Get the provider
    let provider = ctx.registry.default_provider().ok_or_else(|| {
        anyhow::anyhow!("No provider configured. Set ANTHROPIC_API_KEY or OPENAI_API_KEY.")
    })?;

    // Add user message
    let user_msg = Message::user(input);
    store.append_message(session_id, &user_msg).await?;
    messages.push(user_msg);

    // Determine model
    let model = ctx.config.general.model.clone();

    // Build request
    let request = CompletionRequest {
        model,
        messages: messages.clone(),
        max_tokens: ctx.config.limits.max_tokens,
        temperature: 0.7,
        system: None,
        tools: vec![],
        stop: vec![],
    };

    // Stream the response
    println!();
    let mut stream = provider.complete_stream(request).await?;
    let mut response_content = String::new();

    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(chunk) => {
                if let Some(delta) = chunk.delta {
                    print!("{}", delta);
                    io::stdout().flush()?;
                    response_content.push_str(&delta);
                }
            }
            Err(e) => {
                eprintln!("\nStream error: {}", e);
                break;
            }
        }
    }

    println!();

    // Store assistant response
    if !response_content.is_empty() {
        let assistant_msg = Message::assistant(&response_content);
        store.append_message(session_id, &assistant_msg).await?;
        messages.push(assistant_msg);
    }

    Ok(())
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
    println!("║  Press Ctrl+D to exit.                                       ║");
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
