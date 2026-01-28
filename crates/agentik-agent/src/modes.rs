//! Agent modes (execution, planning, etc.).

use serde::{Deserialize, Serialize};

/// Agent operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    /// Full autonomous mode - can plan and execute
    Autonomous,
    /// Planning only - creates task plans, asks for approval
    Planning,
    /// Supervised - asks before each action
    Supervised,
    /// Architect - high-level design without implementation
    Architect,
    /// Ask-only - no code modifications
    AskOnly,
}

impl Default for AgentMode {
    fn default() -> Self {
        Self::Autonomous
    }
}
