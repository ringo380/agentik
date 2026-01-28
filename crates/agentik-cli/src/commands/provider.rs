//! Provider management commands.

use std::sync::Arc;

use crate::{AppContext, ProviderAction};

pub async fn handle(action: ProviderAction, ctx: &Arc<AppContext>) -> anyhow::Result<()> {
    match action {
        ProviderAction::List => {
            println!("Configured providers:\n");
            let providers = ctx.registry.list();
            if providers.is_empty() {
                println!("  No providers configured.");
                println!("\n  Set ANTHROPIC_API_KEY or OPENAI_API_KEY to add providers.");
            } else {
                for provider_id in &providers {
                    if let Some(provider) = ctx.registry.get(provider_id) {
                        let is_default = ctx
                            .registry
                            .default_provider()
                            .map(|p| p.id() == provider.id())
                            .unwrap_or(false);
                        let default_marker = if is_default { " (default)" } else { "" };
                        let status = if provider.is_configured() {
                            "configured"
                        } else {
                            "not configured"
                        };
                        println!("  {} - {}{}", provider.name(), status, default_marker);

                        // Show available models
                        let models = provider.available_models();
                        if !models.is_empty() {
                            println!("    Models:");
                            for model in models.iter().take(3) {
                                println!("      - {} ({})", model.name, model.id);
                            }
                            if models.len() > 3 {
                                println!("      ... and {} more", models.len() - 3);
                            }
                        }
                        println!();
                    }
                }
            }
        }
        ProviderAction::Add { name } => {
            println!("Adding provider: {}", name);
            println!("\nTo add a provider, set the appropriate environment variable:");
            match name.as_str() {
                "anthropic" => {
                    println!("  export ANTHROPIC_API_KEY=your-api-key");
                    println!("\n  Get your API key at: https://console.anthropic.com/");
                }
                "openai" => {
                    println!("  export OPENAI_API_KEY=your-api-key");
                    println!("\n  Get your API key at: https://platform.openai.com/");
                }
                "local" | "ollama" => {
                    println!("  No API key needed for local models.");
                    println!("  Install Ollama: https://ollama.ai");
                    println!("  Then run: ollama pull llama3.2");
                }
                _ => {
                    println!("  Unknown provider: {}", name);
                    println!("  Available providers: anthropic, openai, local");
                }
            }
        }
        ProviderAction::Test { name } => {
            println!("Testing provider: {}\n", name);
            if let Some(provider) = ctx.registry.get(&name) {
                if provider.is_configured() {
                    println!("  ✓ Provider is configured");
                    // TODO: Make a simple test request
                    println!("  Provider: {}", provider.name());
                    println!("  Models available: {}", provider.available_models().len());
                } else {
                    println!("  ✗ Provider is not configured");
                }
            } else {
                println!("  ✗ Provider '{}' not found", name);
                println!("\n  Available providers:");
                for p in ctx.registry.list() {
                    println!("    - {}", p);
                }
            }
        }
    }
    Ok(())
}
