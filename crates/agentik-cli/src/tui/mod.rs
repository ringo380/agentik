//! Terminal UI for interactive mode.

use std::sync::Arc;

use crate::{AppContext, Cli};

pub async fn run(cli: Cli, ctx: Arc<AppContext>) -> anyhow::Result<()> {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!(
        "║  agentik v{}                                              ║",
        env!("CARGO_PKG_VERSION")
    );
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  Type /help for commands, or start chatting.                 ║");
    println!("║  Press Ctrl+C to exit.                                       ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // Show provider info
    if let Some(provider) = ctx.registry.default_provider() {
        let model = cli
            .model
            .as_ref()
            .map(|m| m.as_str())
            .unwrap_or(&ctx.config.general.model);
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
        println!("\nInitial prompt: {}", prompt);
    }

    // TODO: Implement full interactive TUI with ratatui
    println!("\n(Interactive mode not yet fully implemented)");
    println!("Run 'agentik --help' for available options.");
    println!("Use 'agentik -p \"your prompt\"' for print mode.");

    Ok(())
}
