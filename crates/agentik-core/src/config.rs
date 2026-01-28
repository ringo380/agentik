//! Configuration system for Agentik.

use figment::{
    providers::{Env, Format, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::Error;

/// Main configuration struct for Agentik.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// General settings
    pub general: GeneralConfig,
    /// Display settings
    pub display: DisplayConfig,
    /// Resource limits
    pub limits: LimitsConfig,
    /// Permission settings
    pub permissions: PermissionsConfig,
    /// Sandbox settings
    pub sandbox: SandboxConfig,
    /// Provider configurations
    pub providers: ProvidersConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            display: DisplayConfig::default(),
            limits: LimitsConfig::default(),
            permissions: PermissionsConfig::default(),
            sandbox: SandboxConfig::default(),
            providers: ProvidersConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// Default model to use
    pub model: String,
    /// Default provider
    pub provider: String,
    /// Enable sandbox mode
    pub sandbox: bool,
    /// Auto-save sessions
    pub auto_save: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-20250514".to_string(),
            provider: "anthropic".to_string(),
            sandbox: true,
            auto_save: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    /// Enable syntax highlighting
    pub syntax_highlight: bool,
    /// Show costs after each request
    pub show_costs: bool,
    /// Show token counts
    pub show_tokens: bool,
    /// Render markdown
    pub markdown_render: bool,
    /// Color mode: auto, always, never
    pub color: String,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            syntax_highlight: true,
            show_costs: true,
            show_tokens: true,
            markdown_render: true,
            color: "auto".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LimitsConfig {
    /// Maximum tokens per response
    pub max_tokens: u32,
    /// Maximum files in context
    pub max_context_files: usize,
    /// Maximum turns per session
    pub max_turns: usize,
    /// Cost warning threshold (USD)
    pub cost_warning_threshold: f64,
    /// Daily budget (USD)
    pub daily_budget: Option<f64>,
    /// Monthly budget (USD)
    pub monthly_budget: Option<f64>,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_tokens: 8192,
            max_context_files: 50,
            max_turns: 100,
            cost_warning_threshold: 1.0,
            daily_budget: None,
            monthly_budget: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PermissionsConfig {
    /// Tools allowed by default
    pub default_allow: Vec<String>,
    /// Tools requiring confirmation
    pub require_confirm: Vec<String>,
    /// Tools always denied
    pub always_deny: Vec<String>,
}

impl Default for PermissionsConfig {
    fn default() -> Self {
        Self {
            default_allow: vec![
                "Read".to_string(),
                "Glob".to_string(),
                "Grep".to_string(),
            ],
            require_confirm: vec![
                "Write".to_string(),
                "Edit".to_string(),
                "Bash".to_string(),
            ],
            always_deny: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    /// Enable sandbox
    pub enabled: bool,
    /// Sandbox mode: directory, container, none
    pub mode: String,
    /// Allowed paths outside working directory
    pub allowed_paths: Vec<String>,
    /// Blocked commands
    pub blocked_commands: Vec<String>,
    /// Allow network access
    pub allow_network: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: "directory".to_string(),
            allowed_paths: vec!["~/.cargo".to_string(), "~/.npm".to_string()],
            blocked_commands: vec!["rm -rf /".to_string(), "sudo".to_string()],
            allow_network: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProvidersConfig {
    /// Default provider to use
    pub default_provider: Option<String>,
    /// Anthropic configuration
    pub anthropic: Option<ProviderConfig>,
    /// OpenAI configuration
    pub openai: Option<ProviderConfig>,
    /// Local/Ollama configuration
    pub local: Option<LocalProviderConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    /// API key (can be set directly or via environment)
    pub api_key: Option<String>,
    /// Environment variable name for API key
    pub api_key_env: Option<String>,
    /// Default model for this provider
    pub default_model: Option<String>,
    /// Base URL (optional, for custom endpoints)
    pub base_url: Option<String>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            api_key_env: None,
            default_model: None,
            base_url: None,
        }
    }
}

impl ProviderConfig {
    /// Resolve the API key from either direct value or environment variable.
    pub fn resolve_api_key(&self) -> Option<String> {
        // First try direct api_key
        if let Some(ref key) = self.api_key {
            return Some(key.clone());
        }
        // Then try environment variable
        if let Some(ref env_var) = self.api_key_env {
            if let Ok(key) = std::env::var(env_var) {
                return Some(key);
            }
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalProviderConfig {
    /// Enable local provider
    pub enabled: bool,
    /// Ollama URL
    pub base_url: Option<String>,
    /// Default model
    pub default_model: Option<String>,
}

impl Default for LocalProviderConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            base_url: None,
            default_model: None,
        }
    }
}

/// Validation result with multiple issues.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// List of validation issues
    pub issues: Vec<ValidationIssue>,
}

impl ValidationResult {
    /// Create a new empty validation result.
    pub fn new() -> Self {
        Self { issues: Vec::new() }
    }

    /// Check if validation passed (no errors).
    pub fn is_ok(&self) -> bool {
        !self.issues.iter().any(|i| i.severity == IssueSeverity::Error)
    }

    /// Get only error-level issues.
    pub fn errors(&self) -> Vec<&ValidationIssue> {
        self.issues.iter().filter(|i| i.severity == IssueSeverity::Error).collect()
    }

    /// Get only warning-level issues.
    pub fn warnings(&self) -> Vec<&ValidationIssue> {
        self.issues.iter().filter(|i| i.severity == IssueSeverity::Warning).collect()
    }

    /// Add an issue to the result.
    pub fn add(&mut self, issue: ValidationIssue) {
        self.issues.push(issue);
    }

    /// Add an error.
    pub fn add_error(&mut self, field: impl Into<String>, message: impl Into<String>) {
        self.issues.push(ValidationIssue {
            severity: IssueSeverity::Error,
            field: field.into(),
            message: message.into(),
        });
    }

    /// Add a warning.
    pub fn add_warning(&mut self, field: impl Into<String>, message: impl Into<String>) {
        self.issues.push(ValidationIssue {
            severity: IssueSeverity::Warning,
            field: field.into(),
            message: message.into(),
        });
    }
}

impl Default for ValidationResult {
    fn default() -> Self {
        Self::new()
    }
}

/// A single validation issue.
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    /// Severity of the issue
    pub severity: IssueSeverity,
    /// Field path (e.g., "limits.max_tokens")
    pub field: String,
    /// Human-readable message
    pub message: String,
}

/// Severity level for validation issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueSeverity {
    /// Warnings don't prevent loading
    Warning,
    /// Errors prevent loading
    Error,
}

impl Config {
    /// Load configuration from all sources.
    pub fn load() -> Result<Self, figment::Error> {
        let config_dir = Self::config_dir();
        let project_config = PathBuf::from(".agentik/config.toml");

        Figment::new()
            // Default values
            .merge(figment::providers::Serialized::defaults(Config::default()))
            // User config
            .merge(Toml::file(config_dir.join("config.toml")))
            // Project config
            .merge(Toml::file(&project_config))
            // Project local config (gitignored)
            .merge(Toml::file(".agentik/config.local.toml"))
            // Environment variables
            .merge(Env::prefixed("AGENTIK_").split("_"))
            .extract()
    }

    /// Load and validate configuration.
    pub fn load_validated() -> Result<Self, Error> {
        let config = Self::load().map_err(|e| Error::Config(e.to_string()))?;
        let result = config.validate();

        if !result.is_ok() {
            let errors: Vec<String> = result
                .errors()
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect();
            return Err(Error::Config(format!("Configuration validation failed:\n  {}", errors.join("\n  "))));
        }

        // Log warnings
        for warning in result.warnings() {
            tracing::warn!("Config warning - {}: {}", warning.field, warning.message);
        }

        Ok(config)
    }

    /// Validate the configuration.
    pub fn validate(&self) -> ValidationResult {
        let mut result = ValidationResult::new();

        // Validate general settings
        if self.general.model.is_empty() {
            result.add_error("general.model", "Model name cannot be empty");
        }

        if self.general.provider.is_empty() {
            result.add_error("general.provider", "Provider name cannot be empty");
        }

        // Validate limits
        if self.limits.max_tokens == 0 {
            result.add_error("limits.max_tokens", "max_tokens must be greater than 0");
        }

        if self.limits.max_tokens > 200_000 {
            result.add_warning("limits.max_tokens", "max_tokens is very high (> 200k), this may cause issues");
        }

        if self.limits.max_context_files == 0 {
            result.add_error("limits.max_context_files", "max_context_files must be greater than 0");
        }

        if self.limits.max_turns == 0 {
            result.add_error("limits.max_turns", "max_turns must be greater than 0");
        }

        if self.limits.cost_warning_threshold < 0.0 {
            result.add_error("limits.cost_warning_threshold", "cost_warning_threshold cannot be negative");
        }

        if let Some(budget) = self.limits.daily_budget {
            if budget < 0.0 {
                result.add_error("limits.daily_budget", "daily_budget cannot be negative");
            }
        }

        if let Some(budget) = self.limits.monthly_budget {
            if budget < 0.0 {
                result.add_error("limits.monthly_budget", "monthly_budget cannot be negative");
            }
        }

        // Validate display settings
        let valid_color_modes = ["auto", "always", "never"];
        if !valid_color_modes.contains(&self.display.color.as_str()) {
            result.add_error(
                "display.color",
                format!("Invalid color mode '{}'. Valid values: {:?}", self.display.color, valid_color_modes)
            );
        }

        // Validate sandbox settings
        let valid_sandbox_modes = ["directory", "container", "none"];
        if !valid_sandbox_modes.contains(&self.sandbox.mode.as_str()) {
            result.add_error(
                "sandbox.mode",
                format!("Invalid sandbox mode '{}'. Valid values: {:?}", self.sandbox.mode, valid_sandbox_modes)
            );
        }

        // Validate provider configurations
        if let Some(ref anthropic) = self.providers.anthropic {
            if anthropic.api_key.as_ref().map(|k| k.is_empty()).unwrap_or(false) {
                result.add_warning("providers.anthropic.api_key", "API key is empty string");
            }
        }

        if let Some(ref openai) = self.providers.openai {
            if openai.api_key.as_ref().map(|k| k.is_empty()).unwrap_or(false) {
                result.add_warning("providers.openai.api_key", "API key is empty string");
            }
            if let Some(ref base_url) = openai.base_url {
                if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
                    result.add_error(
                        "providers.openai.base_url",
                        "base_url must start with http:// or https://"
                    );
                }
            }
        }

        result
    }

    /// Get the configuration directory.
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .map(|p| p.join("agentik"))
            .unwrap_or_else(|| PathBuf::from("~/.config/agentik"))
    }

    /// Get the data directory (for sessions, etc.).
    pub fn data_dir() -> PathBuf {
        dirs::data_dir()
            .map(|p| p.join("agentik"))
            .unwrap_or_else(|| PathBuf::from("~/.local/share/agentik"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_valid() {
        let config = Config::default();
        let result = config.validate();
        assert!(result.is_ok(), "Default config should be valid: {:?}", result.issues);
    }

    #[test]
    fn test_invalid_max_tokens() {
        let mut config = Config::default();
        config.limits.max_tokens = 0;
        let result = config.validate();
        assert!(!result.is_ok());
        assert!(result.errors().iter().any(|e| e.field == "limits.max_tokens"));
    }

    #[test]
    fn test_invalid_color_mode() {
        let mut config = Config::default();
        config.display.color = "invalid".to_string();
        let result = config.validate();
        assert!(!result.is_ok());
        assert!(result.errors().iter().any(|e| e.field == "display.color"));
    }

    #[test]
    fn test_invalid_sandbox_mode() {
        let mut config = Config::default();
        config.sandbox.mode = "invalid".to_string();
        let result = config.validate();
        assert!(!result.is_ok());
        assert!(result.errors().iter().any(|e| e.field == "sandbox.mode"));
    }

    #[test]
    fn test_negative_budget_is_error() {
        let mut config = Config::default();
        config.limits.daily_budget = Some(-10.0);
        let result = config.validate();
        assert!(!result.is_ok());
        assert!(result.errors().iter().any(|e| e.field == "limits.daily_budget"));
    }

    #[test]
    fn test_high_max_tokens_is_warning() {
        let mut config = Config::default();
        config.limits.max_tokens = 500_000;
        let result = config.validate();
        assert!(result.is_ok()); // Warnings don't fail validation
        assert!(result.warnings().iter().any(|e| e.field == "limits.max_tokens"));
    }
}
