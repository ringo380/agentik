//! # agentik-cli
//!
//! Command-line interface for Agentik.

use std::sync::Arc;

use clap::{Parser, Subcommand};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use agentik_core::Config;
use agentik_providers::ProviderRegistry;

mod commands;
mod output;
mod tui;

/// Application context containing shared state.
pub struct AppContext {
    pub config: Config,
    pub registry: ProviderRegistry,
}

/// Agentik - CLI-based agentic AI tool
#[derive(Parser)]
#[command(name = "agentik")]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Initial prompt to send (starts interactive mode after)
    #[arg(value_name = "PROMPT")]
    prompt: Option<String>,

    /// Print mode - execute prompt and exit (non-interactive)
    #[arg(short, long)]
    print: bool,

    /// Continue most recent session
    #[arg(short, long)]
    r#continue: bool,

    /// Resume specific session by ID
    #[arg(short, long, value_name = "SESSION_ID")]
    resume: Option<String>,

    /// Model to use (e.g., opus, sonnet, gpt-4o)
    #[arg(short, long)]
    model: Option<String>,

    /// Provider to use (anthropic, openai, local)
    #[arg(long)]
    provider: Option<String>,

    /// Add file or directory to context
    #[arg(short, long, value_name = "PATH")]
    add: Vec<String>,

    /// Start in planning mode
    #[arg(long)]
    plan: bool,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Disable color output
    #[arg(long)]
    no_color: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Session management
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Configuration management
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// MCP server management
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
    /// Provider management
    Provider {
        #[command(subcommand)]
        action: ProviderAction,
    },
    /// Show version information
    Version,
    /// Diagnose installation issues
    Doctor,
}

#[derive(Subcommand)]
enum SessionAction {
    /// List all sessions
    List {
        /// Maximum sessions to show
        #[arg(short, long, default_value = "20")]
        limit: usize,
        /// Filter by status
        #[arg(short, long)]
        filter: Option<String>,
    },
    /// Show session details
    Show {
        /// Session ID
        id: String,
    },
    /// Export session to file
    Export {
        /// Session ID
        id: String,
        /// Output format (json, markdown, html)
        #[arg(short, long, default_value = "json")]
        format: String,
    },
    /// Delete session
    Delete {
        /// Session ID
        id: String,
    },
    /// Set or show session title
    Title {
        /// Session ID
        id: String,
        /// Title to set (omit to show current title)
        title: Option<String>,
    },
    /// Manage session tags
    Tag {
        /// Session ID
        id: String,
        #[command(subcommand)]
        action: TagAction,
    },
}

#[derive(Subcommand)]
pub enum TagAction {
    /// Add a tag to the session
    Add {
        /// Tag to add
        tag: String,
    },
    /// Remove a tag from the session
    Remove {
        /// Tag to remove
        tag: String,
    },
    /// List all tags
    List,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show current configuration
    Show,
    /// Set a configuration value
    Set {
        /// Configuration key
        key: String,
        /// Value to set
        value: String,
    },
    /// Open config in editor
    Edit,
    /// Reset to defaults
    Reset,
}

#[derive(Subcommand)]
enum McpAction {
    /// List configured MCP servers
    List,
    /// Add an MCP server
    Add {
        /// Server name
        name: String,
        /// Command or URL
        target: String,
    },
    /// Remove an MCP server
    Remove {
        /// Server name
        name: String,
    },
    /// Enable an MCP server
    Enable {
        /// Server name
        name: String,
    },
    /// Disable an MCP server
    Disable {
        /// Server name
        name: String,
    },
    /// View server logs
    Logs {
        /// Server name
        name: String,
    },
}

#[derive(Subcommand)]
enum ProviderAction {
    /// List configured providers
    List,
    /// Add a provider
    Add {
        /// Provider name
        name: String,
    },
    /// Test provider connection
    Test {
        /// Provider name
        name: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();

    // Load and validate configuration
    let config = match Config::load_validated() {
        Ok(c) => c,
        Err(e) => {
            // If validation fails, log the error but try to continue with defaults
            tracing::error!("Configuration error: {}", e);
            tracing::warn!("Using default configuration");
            Config::default()
        }
    };

    // Initialize provider registry
    let registry = ProviderRegistry::from_config(&config);

    // Create application context
    let ctx = Arc::new(AppContext { config, registry });

    // Handle subcommands
    match cli.command {
        Some(Commands::Session { action }) => {
            commands::session::handle(action).await?;
        }
        Some(Commands::Config { action }) => {
            commands::config::handle(action).await?;
        }
        Some(Commands::Mcp { action }) => {
            commands::mcp::handle(action).await?;
        }
        Some(Commands::Provider { action }) => {
            commands::provider::handle(action, &ctx).await?;
        }
        Some(Commands::Version) => {
            println!("agentik {}", env!("CARGO_PKG_VERSION"));
        }
        Some(Commands::Doctor) => {
            commands::doctor::run(&ctx).await?;
        }
        None => {
            // Interactive mode
            if cli.print {
                // Print mode - single response then exit
                if let Some(ref prompt) = cli.prompt {
                    commands::print::run(prompt, &cli, &ctx).await?;
                } else {
                    anyhow::bail!("Print mode requires a prompt");
                }
            } else {
                // Interactive REPL mode
                tui::run(cli, ctx).await?;
            }
        }
    }

    Ok(())
}
