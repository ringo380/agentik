//! Diagnostic command to check installation.

use std::sync::Arc;

use agentik_providers::LocalProvider;

use crate::AppContext;

pub async fn run(ctx: &Arc<AppContext>) -> anyhow::Result<()> {
    println!("Running diagnostics...\n");

    // Check config directory
    let config_dir = agentik_core::Config::config_dir();
    println!("Config directory: {:?}", config_dir);
    if config_dir.exists() {
        println!("  ✓ Exists");
    } else {
        println!("  ✗ Does not exist (will be created on first use)");
    }

    // Check data directory
    let data_dir = agentik_core::Config::data_dir();
    println!("\nData directory: {:?}", data_dir);
    if data_dir.exists() {
        println!("  ✓ Exists");
    } else {
        println!("  ✗ Does not exist (will be created on first use)");
    }

    // Check providers
    println!("\nProviders:");
    let providers = ctx.registry.list();
    if providers.is_empty() {
        println!("  ✗ No providers configured");
    } else {
        for provider_id in &providers {
            if let Some(provider) = ctx.registry.get(provider_id) {
                let status = if provider.is_configured() {
                    "✓ configured"
                } else {
                    "✗ not configured"
                };
                println!("  {} {} ({})", status, provider.name(), provider_id);
            }
        }
    }

    // Check default provider
    if let Some(default) = ctx.registry.default_provider() {
        println!("\nDefault provider: {} ({})", default.name(), default.id());
    } else {
        println!("\nDefault provider: ✗ none");
    }

    // Check environment variables
    println!("\nAPI Keys:");
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        println!("  ✓ ANTHROPIC_API_KEY is set");
    } else {
        println!("  ✗ ANTHROPIC_API_KEY is not set");
    }

    if std::env::var("OPENAI_API_KEY").is_ok() {
        println!("  ✓ OPENAI_API_KEY is set");
    } else {
        println!("  ✗ OPENAI_API_KEY is not set");
    }

    // Check Ollama
    println!("\nLocal Models (Ollama):");
    let ollama = LocalProvider::new();
    if ollama.is_running().await {
        println!("  ✓ Ollama is running");
        match ollama.list_models().await {
            Ok(models) => {
                if models.is_empty() {
                    println!("  ✗ No models installed");
                } else {
                    println!("  Available models:");
                    for model in models.iter().take(5) {
                        let size_mb = model.size / 1024 / 1024;
                        println!("    - {} ({} MB)", model.name, size_mb);
                    }
                    if models.len() > 5 {
                        println!("    ... and {} more", models.len() - 5);
                    }
                }
            }
            Err(e) => {
                println!("  ✗ Failed to list models: {}", e);
            }
        }
    } else {
        println!("  ✗ Ollama is not running");
        println!("    Install from: https://ollama.ai");
    }

    println!("\nDiagnostics complete.");
    Ok(())
}
