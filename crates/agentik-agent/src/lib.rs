//! # agentik-agent
//!
//! Agent orchestration and planning for Agentik.
//!
//! This crate provides:
//! - Core agent loop
//! - Planning mode
//! - Architect/Editor model separation
//! - "Anytime" question asking system
//! - Tool execution orchestration

pub mod agent;
pub mod executor;
pub mod modes;
pub mod planning;
pub mod questions;

pub use agent::{
    Agent, AgentBuilder, AgentConfig, AgentError, AgentEventHandler, AgentResponse, AgentResult,
    NoOpEventHandler, StepResult, TurnUsage,
};
pub use executor::{
    AutoApproveHandler, DenialReason, DenyAllHandler, ExecutorBuilder, PermissionHandler,
    ToolExecutor,
};
pub use modes::AgentMode;
pub use planning::PlanningState;
pub use questions::QuestionQueue;
