//! # agentik-metrics
//!
//! Usage tracking and cost management for Agentik.
//!
//! This crate provides:
//! - Token and cost tracking per request
//! - Session, daily, monthly aggregation
//! - Budget limits with warnings
//! - Model benchmark scores and rankings

pub mod analytics;
pub mod benchmarks;
pub mod cutoff;
pub mod usage;

pub use analytics::UsageAnalytics;
pub use benchmarks::ModelBenchmarks;
pub use cutoff::BudgetEnforcer;
pub use usage::UsageTracker;
