//! # agentik-providers
//!
//! Multi-provider AI abstraction layer for Agentik.
//!
//! This crate provides:
//! - Provider trait for abstracting AI providers
//! - Implementations for Anthropic, OpenAI, and local models (Ollama)
//! - Tool calling normalization across providers
//! - Streaming support
//! - Model registry and selection

pub mod anthropic;
pub mod local;
pub mod openai;
pub mod registry;
pub mod traits;

pub use anthropic::AnthropicProvider;
pub use local::LocalProvider;
pub use openai::OpenAIProvider;
pub use registry::ProviderRegistry;
pub use traits::{CompletionRequest, CompletionResponse, ModelInfo, Pricing, Provider, StreamChunk, ToolCapable};
