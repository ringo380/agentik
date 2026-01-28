//! Session recovery functionality.
//!
//! Provides functionality for resuming sessions, finding sessions by various
//! criteria, and validating session integrity.

use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use thiserror::Error;

use agentik_core::{Session, SessionState};

use crate::store::{SessionQuery, SessionStore, SessionSummary, SqliteSessionStore, StoreError};

/// Errors that can occur during session recovery.
#[derive(Error, Debug)]
pub enum RecoveryError {
    #[error("No sessions found")]
    NoSessionsFound,

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Ambiguous session prefix '{0}': matches {1} sessions")]
    AmbiguousPrefix(String, usize),

    #[error("Session is in invalid state for resume: {0:?}")]
    InvalidState(SessionState),

    #[error("Session validation failed: {0}")]
    ValidationFailed(String),

    #[error("Store error: {0}")]
    Store(#[from] StoreError),

    #[error("No session found for directory: {0}")]
    NoSessionForDirectory(PathBuf),
}

pub type Result<T> = std::result::Result<T, RecoveryError>;

/// Options for session recovery.
#[derive(Debug, Clone, Default)]
pub struct RecoveryOptions {
    /// Whether to validate message integrity.
    pub validate_messages: bool,
    /// Whether to repair corrupted sessions.
    pub repair: bool,
    /// Only consider sessions in these states.
    pub allowed_states: Option<Vec<SessionState>>,
}

impl RecoveryOptions {
    /// Create options that allow any resumable state.
    pub fn resumable() -> Self {
        Self {
            allowed_states: Some(vec![
                SessionState::Active,
                SessionState::Suspended,
                SessionState::Sleeping,
            ]),
            ..Default::default()
        }
    }

    /// Create options with validation enabled.
    pub fn with_validation() -> Self {
        Self {
            validate_messages: true,
            ..Default::default()
        }
    }
}

/// Session recovery manager.
pub struct SessionRecovery<S: SessionStore> {
    store: S,
}

impl SessionRecovery<SqliteSessionStore> {
    /// Create a recovery manager with the default store location.
    pub fn open_default() -> std::result::Result<Self, StoreError> {
        let store = SqliteSessionStore::open_default()?;
        Ok(Self { store })
    }
}

impl<S: SessionStore> SessionRecovery<S> {
    /// Create a new recovery manager with the given store.
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Get a reference to the underlying store.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Resume the most recently active session.
    ///
    /// This is used for the `--continue` / `-c` flag.
    pub async fn resume_most_recent(&self) -> Result<Session> {
        let summary = self
            .store
            .get_most_recent()
            .await?
            .ok_or(RecoveryError::NoSessionsFound)?;

        // Check if session is in a resumable state
        if !self.is_resumable_state(summary.state) {
            return Err(RecoveryError::InvalidState(summary.state));
        }

        let session = self.store.get(&summary.id).await?;
        Ok(session)
    }

    /// Resume a session by its full ID.
    pub async fn resume(&self, id: &str) -> Result<Session> {
        let session = self
            .store
            .get(id)
            .await
            .map_err(|_| RecoveryError::SessionNotFound(id.to_string()))?;

        if !self.is_resumable_state(session.metadata.state) {
            return Err(RecoveryError::InvalidState(session.metadata.state));
        }

        Ok(session)
    }

    /// Resume a session by ID prefix.
    ///
    /// This allows users to specify partial session IDs like `abc123` instead
    /// of the full UUID.
    pub async fn resume_by_prefix(&self, prefix: &str) -> Result<Session> {
        let matches = self.store.find_by_prefix(prefix).await?;

        match matches.len() {
            0 => Err(RecoveryError::SessionNotFound(prefix.to_string())),
            1 => {
                let summary = &matches[0];
                if !self.is_resumable_state(summary.state) {
                    return Err(RecoveryError::InvalidState(summary.state));
                }
                let session = self.store.get(&summary.id).await?;
                Ok(session)
            }
            n => Err(RecoveryError::AmbiguousPrefix(prefix.to_string(), n)),
        }
    }

    /// Find sessions for a specific directory.
    ///
    /// Returns sessions that were created in the given working directory,
    /// sorted by most recent first.
    pub async fn find_for_directory(&self, path: impl AsRef<Path>) -> Result<Vec<SessionSummary>> {
        let path = path.as_ref().to_path_buf();
        let query = SessionQuery::new().with_directory(path.clone());
        let sessions = self.store.list(&query).await?;

        if sessions.is_empty() {
            return Err(RecoveryError::NoSessionForDirectory(path));
        }

        Ok(sessions)
    }

    /// Get the most recent session for a directory.
    pub async fn get_most_recent_for_directory(&self, path: impl AsRef<Path>) -> Result<Session> {
        let sessions = self.find_for_directory(path).await?;
        let summary = sessions.first().ok_or(RecoveryError::NoSessionsFound)?;

        if !self.is_resumable_state(summary.state) {
            return Err(RecoveryError::InvalidState(summary.state));
        }

        let session = self.store.get(&summary.id).await?;
        Ok(session)
    }

    /// Validate a session's integrity.
    pub async fn validate(&self, id: &str, options: &RecoveryOptions) -> Result<ValidationResult> {
        let session = self
            .store
            .get(id)
            .await
            .map_err(|_| RecoveryError::SessionNotFound(id.to_string()))?;

        let mut result = ValidationResult {
            session_id: id.to_string(),
            is_valid: true,
            issues: Vec::new(),
        };

        // Check state
        if let Some(ref allowed) = options.allowed_states {
            if !allowed.contains(&session.metadata.state) {
                result.issues.push(ValidationIssue {
                    severity: IssueSeverity::Warning,
                    description: format!(
                        "Session state {:?} not in allowed states",
                        session.metadata.state
                    ),
                });
            }
        }

        // Check message count consistency
        let stored_count = session.metadata.metrics.turn_count as usize;
        let actual_count = session.messages.len();
        if stored_count != actual_count && stored_count > 0 {
            result.issues.push(ValidationIssue {
                severity: IssueSeverity::Warning,
                description: format!(
                    "Message count mismatch: metadata says {}, found {}",
                    stored_count, actual_count
                ),
            });
        }

        // Check compaction boundary
        if session.compact_boundary > session.messages.len() {
            result.issues.push(ValidationIssue {
                severity: IssueSeverity::Error,
                description: format!(
                    "Compaction boundary {} exceeds message count {}",
                    session.compact_boundary,
                    session.messages.len()
                ),
            });
            result.is_valid = false;
        }

        // Check if summary exists but boundary is 0
        if session.summary.is_some() && session.compact_boundary == 0 {
            result.issues.push(ValidationIssue {
                severity: IssueSeverity::Warning,
                description: "Summary exists but compaction boundary is 0".to_string(),
            });
        }

        // Validate messages if requested
        if options.validate_messages {
            for (idx, msg) in session.messages.iter().enumerate() {
                if msg.id.is_empty() {
                    result.issues.push(ValidationIssue {
                        severity: IssueSeverity::Error,
                        description: format!("Message {} has empty ID", idx),
                    });
                    result.is_valid = false;
                }
            }
        }

        Ok(result)
    }

    /// Archive old sessions that haven't been active for a given duration.
    pub async fn archive_old_sessions(&self, older_than_days: u32) -> Result<Vec<String>> {
        let cutoff = Utc::now() - Duration::days(older_than_days as i64);
        let query = SessionQuery::new().with_limit(1000);
        let sessions = self.store.list(&query).await?;

        let mut archived = Vec::new();

        for session in sessions {
            if session.last_active_at < cutoff && session.state != SessionState::Archived {
                self.store
                    .set_state(&session.id, SessionState::Archived)
                    .await?;
                archived.push(session.id);
            }
        }

        Ok(archived)
    }

    /// List recent sessions for display.
    pub async fn list_recent(&self, limit: usize) -> Result<Vec<SessionSummary>> {
        let query = SessionQuery::new().with_limit(limit);
        let sessions = self.store.list(&query).await?;
        Ok(sessions)
    }

    /// List sessions filtered by state.
    pub async fn list_by_state(
        &self,
        state: SessionState,
        limit: usize,
    ) -> Result<Vec<SessionSummary>> {
        let query = SessionQuery::new().with_state(state).with_limit(limit);
        let sessions = self.store.list(&query).await?;
        Ok(sessions)
    }

    /// Check if a state is resumable.
    fn is_resumable_state(&self, state: SessionState) -> bool {
        matches!(
            state,
            SessionState::Active | SessionState::Suspended | SessionState::Sleeping
        )
    }

    /// Smart resume: try prefix first, fall back to most recent if not found.
    pub async fn smart_resume(&self, id_or_prefix: Option<&str>) -> Result<Session> {
        match id_or_prefix {
            Some(id) => {
                // Try exact match first
                if let Ok(session) = self.resume(id).await {
                    return Ok(session);
                }
                // Fall back to prefix match
                self.resume_by_prefix(id).await
            }
            None => self.resume_most_recent().await,
        }
    }
}

/// Result of session validation.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Session ID that was validated.
    pub session_id: String,
    /// Whether the session is valid.
    pub is_valid: bool,
    /// List of issues found.
    pub issues: Vec<ValidationIssue>,
}

/// An issue found during validation.
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    /// Severity of the issue.
    pub severity: IssueSeverity,
    /// Description of the issue.
    pub description: String,
}

/// Severity of a validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueSeverity {
    /// Informational, no action needed.
    Info,
    /// Warning, session may work but has issues.
    Warning,
    /// Error, session may be corrupted.
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SqliteSessionStore;
    use tempfile::TempDir;

    async fn create_test_recovery() -> (SessionRecovery<SqliteSessionStore>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let store = SqliteSessionStore::new(temp_dir.path()).unwrap();
        let recovery = SessionRecovery::new(store);
        (recovery, temp_dir)
    }

    #[tokio::test]
    async fn test_resume_most_recent_no_sessions() {
        let (recovery, _tmp) = create_test_recovery().await;

        let result = recovery.resume_most_recent().await;
        assert!(matches!(result, Err(RecoveryError::NoSessionsFound)));
    }

    #[tokio::test]
    async fn test_resume_most_recent() {
        let (recovery, _tmp) = create_test_recovery().await;

        // Create a session
        let session = Session::new(PathBuf::from("/tmp/test"));
        recovery.store.create(&session).await.unwrap();

        // Resume it
        let resumed = recovery.resume_most_recent().await.unwrap();
        assert_eq!(resumed.id(), session.id());
    }

    #[tokio::test]
    async fn test_resume_by_prefix() {
        let (recovery, _tmp) = create_test_recovery().await;

        // Create a session
        let session = Session::new(PathBuf::from("/tmp/test"));
        let prefix = &session.id()[..8];
        recovery.store.create(&session).await.unwrap();

        // Resume by prefix
        let resumed = recovery.resume_by_prefix(prefix).await.unwrap();
        assert_eq!(resumed.id(), session.id());
    }

    #[tokio::test]
    async fn test_resume_archived_fails() {
        let (recovery, _tmp) = create_test_recovery().await;

        // Create and archive a session
        let session = Session::new(PathBuf::from("/tmp/test"));
        recovery.store.create(&session).await.unwrap();
        recovery
            .store
            .set_state(session.id(), SessionState::Archived)
            .await
            .unwrap();

        // Should fail to resume archived session
        let result = recovery.resume(session.id()).await;
        assert!(matches!(result, Err(RecoveryError::InvalidState(_))));
    }

    #[tokio::test]
    async fn test_find_for_directory() {
        let (recovery, _tmp) = create_test_recovery().await;

        let dir = PathBuf::from("/tmp/test-project");
        let session = Session::new(dir.clone());
        recovery.store.create(&session).await.unwrap();

        let found = recovery.find_for_directory(&dir).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, session.id());
    }

    #[tokio::test]
    async fn test_validate_session() {
        let (recovery, _tmp) = create_test_recovery().await;

        let session = Session::new(PathBuf::from("/tmp/test"));
        recovery.store.create(&session).await.unwrap();

        let options = RecoveryOptions::with_validation();
        let result = recovery.validate(session.id(), &options).await.unwrap();

        assert!(result.is_valid);
    }

    #[tokio::test]
    async fn test_smart_resume() {
        let (recovery, _tmp) = create_test_recovery().await;

        // Create a session
        let session = Session::new(PathBuf::from("/tmp/test"));
        recovery.store.create(&session).await.unwrap();

        // Smart resume with no ID should get most recent
        let resumed = recovery.smart_resume(None).await.unwrap();
        assert_eq!(resumed.id(), session.id());

        // Smart resume with prefix should work
        let prefix = &session.id()[..8];
        let resumed = recovery.smart_resume(Some(prefix)).await.unwrap();
        assert_eq!(resumed.id(), session.id());
    }

    #[tokio::test]
    async fn test_list_recent() {
        let (recovery, _tmp) = create_test_recovery().await;

        // Create multiple sessions
        for i in 0..5 {
            let session = Session::new(PathBuf::from(format!("/tmp/test{}", i)));
            recovery.store.create(&session).await.unwrap();
        }

        let recent = recovery.list_recent(3).await.unwrap();
        assert_eq!(recent.len(), 3);
    }
}
