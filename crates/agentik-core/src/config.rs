//! Configuration system for Agentik.

use figment::{
    providers::{Env, Format, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
