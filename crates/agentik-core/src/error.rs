//! Error types for Agentik.
//!
//! This module provides a comprehensive error hierarchy for Agentik,
//! with structured errors that include context and recovery suggestions.

use thiserror::Error;

/// Result type alias using AgentikError.
pub type Result<T> = std::result::Result<T, Error>;

/// Main error type for Agentik.
#[derive(Error, Debug)]
pub enum Error {
    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Provider error with structured details
    #[error("{0}")]
    Provider(#[from] ProviderError),

    /// Session error
    #[error("Session error: {0}")]
    Session(String),

    /// Tool execution error
    #[error("Tool error: {0}")]
    Tool(String),

    /// MCP error
    #[error("MCP error: {0}")]
    Mcp(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Database error
    #[error("Database error: {0}")]
    Database(String),

    /// HTTP request error
    #[error("HTTP error: {0}")]
    Http(String),

    /// Validation error
    #[error("Validation error: {0}")]
    Validation(String),

    /// Not found error
    #[error("Not found: {0}")]
    NotFound(String),

    /// Permission denied
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),

    /// Rate limit exceeded
    #[error("Rate limit exceeded: {message}")]
    RateLimitExceeded {
        message: String,
        retry_after: Option<std::time::Duration>,
    },

    /// Context window exceeded
    #[error("Context window exceeded: {0} tokens, max is {1}")]
    ContextExceeded(u64, u64),

    /// Budget exceeded
    #[error("Budget exceeded: spent ${spent:.2}, limit ${limit:.2}")]
    BudgetExceeded { spent: f64, limit: f64 },
}

impl Error {
    /// Get a recovery suggestion for this error.
    pub fn recovery_suggestion(&self) -> Option<&'static str> {
        match self {
            Error::Config(_) => Some("Check your config file at ~/.config/agentik/config.toml"),
            Error::Provider(e) => e.recovery_suggestion(),
            Error::Session(_) => Some("Try starting a new session with 'agentik'"),
            Error::NotFound(_) => Some("Use 'agentik session list' to see available sessions"),
            Error::PermissionDenied(_) => Some("Check file permissions or use sudo if appropriate"),
            Error::RateLimitExceeded { .. } => Some("Wait a moment and try again"),
            Error::ContextExceeded(_, _) => Some("Try using /compact to summarize older messages"),
            Error::BudgetExceeded { .. } => Some("Increase budget in config or start a new day"),
            _ => None,
        }
    }

    /// Create a provider-not-configured error.
    pub fn provider_not_configured(provider: &str) -> Self {
        Error::Provider(ProviderError::NotConfigured {
            provider: provider.to_string(),
            env_var: match provider {
                "anthropic" => Some("ANTHROPIC_API_KEY".to_string()),
                "openai" => Some("OPENAI_API_KEY".to_string()),
                _ => None,
            },
        })
    }
}

/// Provider-specific errors with detailed context.
#[derive(Error, Debug)]
pub enum ProviderError {
    /// Provider not configured
    #[error("Provider '{provider}' is not configured")]
    NotConfigured {
        provider: String,
        env_var: Option<String>,
    },

    /// Authentication failed
    #[error("Authentication failed for {provider}: {message}")]
    AuthenticationFailed { provider: String, message: String },

    /// API request failed
    #[error("API request to {provider} failed: {status} - {message}")]
    ApiError {
        provider: String,
        status: u16,
        message: String,
    },

    /// Model not found
    #[error("Model '{model}' not found for provider '{provider}'")]
    ModelNotFound { provider: String, model: String },

    /// Streaming error
    #[error("Streaming error from {provider}: {message}")]
    StreamError { provider: String, message: String },

    /// Content filtered
    #[error("Content was filtered by {provider}{}", reason.as_ref().map(|r| format!(": {}", r)).unwrap_or_default())]
    ContentFiltered {
        provider: String,
        reason: Option<String>,
    },

    /// Timeout
    #[error("Request to {provider} timed out after {seconds}s")]
    Timeout { provider: String, seconds: u64 },

    /// Network error
    #[error("Network error connecting to {provider}: {message}")]
    NetworkError { provider: String, message: String },
}

impl ProviderError {
    /// Get a recovery suggestion for this error.
    pub fn recovery_suggestion(&self) -> Option<&'static str> {
        match self {
            ProviderError::NotConfigured {
                env_var: Some(_), ..
            } => Some("Set the API key environment variable"),
            ProviderError::NotConfigured { .. } => {
                Some("Configure the provider in ~/.config/agentik/config.toml")
            }
            ProviderError::AuthenticationFailed { .. } => {
                Some("Check that your API key is valid and not expired")
            }
            ProviderError::ApiError { status: 429, .. } => {
                Some("You've hit rate limits. Wait a moment and try again")
            }
            ProviderError::ApiError {
                status: 500..=599, ..
            } => Some("The API service is having issues. Try again later"),
            ProviderError::ModelNotFound { .. } => Some("Use '/model' to see available models"),
            ProviderError::ContentFiltered { .. } => {
                Some("Rephrase your request to avoid triggering content filters")
            }
            ProviderError::Timeout { .. } => {
                Some("Try a simpler request or check your network connection")
            }
            ProviderError::NetworkError { .. } => Some("Check your internet connection"),
            _ => None,
        }
    }

    /// Create an API error from status code and message.
    pub fn api_error(provider: impl Into<String>, status: u16, message: impl Into<String>) -> Self {
        ProviderError::ApiError {
            provider: provider.into(),
            status,
            message: message.into(),
        }
    }
}

/// Format an error with its recovery suggestion.
pub fn format_error_with_suggestion(error: &Error) -> String {
    let mut output = error.to_string();
    if let Some(suggestion) = error.recovery_suggestion() {
        output.push_str(&format!("\n  Suggestion: {}", suggestion));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_not_configured() {
        let err = Error::provider_not_configured("anthropic");
        assert!(err.to_string().contains("anthropic"));
        assert!(err.recovery_suggestion().is_some());
    }

    #[test]
    fn test_api_error() {
        let err = ProviderError::api_error("openai", 429, "Rate limited");
        assert!(err.to_string().contains("429"));
        assert!(err.recovery_suggestion().is_some());
    }

    #[test]
    fn test_budget_exceeded() {
        let err = Error::BudgetExceeded {
            spent: 10.50,
            limit: 10.0,
        };
        assert!(err.to_string().contains("10.50"));
        assert!(err.recovery_suggestion().is_some());
    }
}
