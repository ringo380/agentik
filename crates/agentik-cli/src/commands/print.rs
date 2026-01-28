//! Print mode (non-interactive single response).

use std::io::{self, Write};
use std::sync::Arc;

use futures::StreamExt;

use agentik_core::Message;
use agentik_providers::CompletionRequest;

use crate::{AppContext, Cli};

pub async fn run(prompt: &str, cli: &Cli, ctx: &Arc<AppContext>) -> anyhow::Result<()> {
    // Get the provider (from CLI arg or default)
    let provider = if let Some(ref provider_name) = cli.provider {
        ctx.registry.get(provider_name).ok_or_else(|| {
            anyhow::anyhow!("Provider '{}' not found. Run 'agentik provider list' to see available providers.", provider_name)
        })?
    } else {
        ctx.registry.default_provider().ok_or_else(|| {
            anyhow::anyhow!("No default provider configured. Set ANTHROPIC_API_KEY or OPENAI_API_KEY.")
        })?
    };

    // Determine model to use
    // If user specified a model, use it; otherwise use provider's first model
    let model = cli.model.clone().unwrap_or_else(|| {
        // Check if config model is available on this provider
        let config_model = &ctx.config.general.model;
        let provider_models = provider.available_models();
        if provider_models.iter().any(|m| m.id == *config_model) {
            config_model.clone()
        } else {
            // Fall back to provider's first available model
            provider_models
                .first()
                .map(|m| m.id.clone())
                .unwrap_or_else(|| config_model.clone())
        }
    });

    // Build the request
    let messages = vec![Message::user(prompt)];

    let request = CompletionRequest {
        model,
        messages,
        max_tokens: ctx.config.limits.max_tokens,
        temperature: 0.7,
        system: None,
        tools: vec![],
        stop: vec![],
    };

    // Stream the response
    let mut stream = provider.complete_stream(request).await?;

    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(chunk) => {
                if let Some(delta) = chunk.delta {
                    print!("{}", delta);
                    io::stdout().flush()?;
                }
            }
            Err(e) => {
                eprintln!("\nError: {}", e);
                break;
            }
        }
    }

    println!();
    Ok(())
}
