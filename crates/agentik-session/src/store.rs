//! Session storage implementation.
//!
//! Provides SQLite-backed storage for session metadata with JSONL files
//! for append-only message logs.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use agentik_core::{
    session::{CompactedSummary, SessionMetrics, SessionState},
    Message, Session, SessionMetadata,
};

/// Errors that can occur during session storage operations.
#[derive(Error, Debug)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Session not found: {0}")]
    NotFound(String),

    #[error("Invalid session state: {0}")]
    InvalidState(String),

    #[error("Storage path error: {0}")]
    PathError(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Query parameters for listing sessions.
#[derive(Debug, Clone, Default)]
pub struct SessionQuery {
    /// Filter by state
    pub state: Option<SessionState>,
    /// Filter by working directory
    pub working_directory: Option<PathBuf>,
    /// Filter by tag
    pub tag: Option<String>,
    /// Full-text search query
    pub search: Option<String>,
    /// Maximum results
    pub limit: usize,
    /// Offset for pagination
    pub offset: usize,
}

impl SessionQuery {
    pub fn new() -> Self {
        Self {
            limit: 50,
            ..Default::default()
        }
    }

    pub fn with_state(mut self, state: SessionState) -> Self {
        self.state = Some(state);
        self
    }

    pub fn with_directory(mut self, dir: PathBuf) -> Self {
        self.working_directory = Some(dir);
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

/// Summary information for session listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: Option<String>,
    pub state: SessionState,
    pub working_directory: PathBuf,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    pub message_count: u32,
    pub total_tokens: u64,
    pub tags: Vec<String>,
}

/// Result of appending a message.
#[derive(Debug)]
pub struct AppendResult {
    pub file_offset: i64,
    pub byte_length: usize,
    pub message_index: i64,
}

/// Aggregated statistics across multiple sessions.
#[derive(Debug, Clone, Default)]
pub struct AggregatedStats {
    pub session_count: u32,
    pub total_tokens_in: u64,
    pub total_tokens_out: u64,
    pub total_cost: f64,
    pub total_turns: u32,
    pub total_tool_calls: u32,
}

/// Session storage trait for abstraction over storage backends.
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Create a new session.
    async fn create(&self, session: &Session) -> Result<()>;

    /// Get a full session by ID (including messages).
    async fn get(&self, id: &str) -> Result<Session>;

    /// Get only session metadata (without messages).
    async fn get_metadata(&self, id: &str) -> Result<SessionMetadata>;

    /// Update session metadata.
    async fn update_metadata(&self, metadata: &SessionMetadata) -> Result<()>;

    /// Delete a session and its messages.
    async fn delete(&self, id: &str) -> Result<()>;

    /// List sessions matching query.
    async fn list(&self, query: &SessionQuery) -> Result<Vec<SessionSummary>>;

    /// Get the most recently active session.
    async fn get_most_recent(&self) -> Result<Option<SessionSummary>>;

    /// Append a message to a session.
    async fn append_message(&self, session_id: &str, message: &Message) -> Result<AppendResult>;

    /// Get messages from a session.
    async fn get_messages(
        &self,
        session_id: &str,
        from: Option<i64>,
        limit: Option<usize>,
    ) -> Result<Vec<Message>>;

    /// Apply compaction to a session.
    async fn apply_compaction(
        &self,
        session_id: &str,
        summary: &CompactedSummary,
        boundary: usize,
    ) -> Result<()>;

    /// Update session state.
    async fn set_state(&self, id: &str, state: SessionState) -> Result<()>;

    /// Update last_active_at timestamp.
    async fn touch(&self, id: &str) -> Result<()>;

    /// Find sessions by ID prefix.
    async fn find_by_prefix(&self, prefix: &str) -> Result<Vec<SessionSummary>>;

    /// Get aggregated statistics, optionally filtered by time.
    async fn get_aggregated_stats(&self, since: Option<DateTime<Utc>>) -> Result<AggregatedStats>;
}

/// SQLite-backed session storage.
pub struct SqliteSessionStore {
    /// Database connection (wrapped in mutex for thread safety).
    conn: Mutex<Connection>,
    /// Base directory for session data (stored for future use).
    #[allow(dead_code)]
    base_dir: PathBuf,
    /// Directory for JSONL message files.
    sessions_dir: PathBuf,
}

impl SqliteSessionStore {
    /// Create a new SQLite session store.
    ///
    /// Creates the database and runs migrations if needed.
    pub fn new(base_dir: impl AsRef<Path>) -> Result<Self> {
        let base_dir = base_dir.as_ref().to_path_buf();
        let sessions_dir = base_dir.join("sessions");

        // Create directories
        fs::create_dir_all(&base_dir)?;
        fs::create_dir_all(&sessions_dir)?;

        // Open database
        let db_path = base_dir.join("sessions.db");
        let conn = Connection::open(&db_path)?;

        // Enable WAL mode for better concurrency
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let store = Self {
            conn: Mutex::new(conn),
            base_dir,
            sessions_dir,
        };

        // Run migrations
        store.run_migrations()?;

        Ok(store)
    }

    /// Open store at the default data directory.
    pub fn open_default() -> Result<Self> {
        let data_dir = dirs::data_local_dir()
            .ok_or_else(|| StoreError::PathError("Could not find data directory".into()))?
            .join("agentik");
        Self::new(data_dir)
    }

    /// Run database migrations.
    fn run_migrations(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Check current version
        let current_version: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 1 {
            // Run initial migration
            let migration = include_str!("../migrations/001_initial.sql");
            conn.execute_batch(migration)?;
        }

        Ok(())
    }

    /// Get the message file path for a session.
    fn message_file_path(&self, session_id: &str) -> PathBuf {
        let session_dir = self.sessions_dir.join(session_id);
        session_dir.join("messages.jsonl")
    }

    /// Ensure session directory exists.
    fn ensure_session_dir(&self, session_id: &str) -> Result<PathBuf> {
        let session_dir = self.sessions_dir.join(session_id);
        fs::create_dir_all(&session_dir)?;
        Ok(session_dir)
    }

    /// Serialize SessionState to string.
    fn state_to_str(state: SessionState) -> &'static str {
        match state {
            SessionState::Active => "active",
            SessionState::Compacting => "compacting",
            SessionState::Suspended => "suspended",
            SessionState::Sleeping => "sleeping",
            SessionState::Archived => "archived",
        }
    }

    /// Parse SessionState from string.
    fn str_to_state(s: &str) -> SessionState {
        match s {
            "active" => SessionState::Active,
            "compacting" => SessionState::Compacting,
            "suspended" => SessionState::Suspended,
            "sleeping" => SessionState::Sleeping,
            "archived" => SessionState::Archived,
            _ => SessionState::Active,
        }
    }

    /// Parse datetime from SQLite string.
    fn parse_datetime(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now())
    }

    /// Format datetime for SQLite.
    fn format_datetime(dt: &DateTime<Utc>) -> String {
        dt.to_rfc3339()
    }
}

#[async_trait]
impl SessionStore for SqliteSessionStore {
    async fn create(&self, session: &Session) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let meta = &session.metadata;

        // Ensure session directory exists
        self.ensure_session_dir(&meta.id)?;
        let message_file = self.message_file_path(&meta.id);

        // Serialize JSON fields
        let git_json = meta.git.as_ref().map(|g| serde_json::to_string(g).unwrap());
        let metrics_json = serde_json::to_string(&meta.metrics)?;
        let model_json = serde_json::to_string(&meta.model)?;
        let summary_json = session
            .summary
            .as_ref()
            .map(|s| serde_json::to_string(s).unwrap());

        conn.execute(
            r#"
            INSERT INTO sessions (
                id, version, state, working_directory, title, parent_session_id,
                created_at, updated_at, last_active_at,
                git_context, metrics, model_config,
                compact_boundary, summary, message_file, message_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            "#,
            params![
                meta.id,
                meta.version,
                Self::state_to_str(meta.state),
                meta.working_directory.to_string_lossy(),
                meta.title,
                meta.parent_session_id,
                Self::format_datetime(&meta.created_at),
                Self::format_datetime(&meta.updated_at),
                Self::format_datetime(&meta.last_active_at),
                git_json,
                metrics_json,
                model_json,
                session.compact_boundary as i64,
                summary_json,
                message_file.to_string_lossy(),
                session.messages.len() as i64,
            ],
        )?;

        // Insert tags
        for tag in &meta.tags {
            conn.execute(
                "INSERT INTO session_tags (session_id, tag) VALUES (?1, ?2)",
                params![meta.id, tag],
            )?;
        }

        // Write initial messages to JSONL file
        if !session.messages.is_empty() {
            let file = File::create(&message_file)?;
            let mut writer = BufWriter::new(file);
            let mut offset: i64 = 0;

            for msg in session.messages.iter() {
                let json = serde_json::to_string(msg)?;
                let bytes = json.as_bytes();

                // Record in message index
                conn.execute(
                    r#"
                    INSERT INTO message_index (
                        session_id, message_id, role, timestamp,
                        file_offset, byte_length, token_count
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                    "#,
                    params![
                        meta.id,
                        msg.id,
                        format!("{:?}", msg.role).to_lowercase(),
                        Self::format_datetime(&msg.timestamp),
                        offset,
                        bytes.len() as i64,
                        msg.token_count,
                    ],
                )?;

                writeln!(writer, "{}", json)?;
                offset += bytes.len() as i64 + 1; // +1 for newline
            }

            writer.flush()?;
        }

        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Session> {
        let metadata = self.get_metadata(id).await?;
        let messages = self.get_messages(id, None, None).await?;

        // Get compaction info
        let conn = self.conn.lock().unwrap();
        let (compact_boundary, summary_json): (i64, Option<String>) = conn.query_row(
            "SELECT compact_boundary, summary FROM sessions WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        let summary = summary_json.map(|s| serde_json::from_str(&s)).transpose()?;

        Ok(Session {
            metadata,
            messages,
            summary,
            compact_boundary: compact_boundary as usize,
        })
    }

    async fn get_metadata(&self, id: &str) -> Result<SessionMetadata> {
        let conn = self.conn.lock().unwrap();

        let row = conn
            .query_row(
                r#"
                SELECT id, version, state, working_directory, title, parent_session_id,
                       created_at, updated_at, last_active_at,
                       git_context, metrics, model_config
                FROM sessions WHERE id = ?1
                "#,
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i32>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(id.to_string()))?;

        let (
            id,
            version,
            state_str,
            working_dir,
            title,
            parent_id,
            created_at,
            updated_at,
            last_active_at,
            git_json,
            metrics_json,
            model_json,
        ) = row;

        // Get tags
        let mut stmt = conn.prepare("SELECT tag FROM session_tags WHERE session_id = ?1")?;
        let tags: Vec<String> = stmt
            .query_map(params![&id], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(SessionMetadata {
            id,
            version: version as u32,
            state: Self::str_to_state(&state_str),
            working_directory: PathBuf::from(working_dir),
            title,
            parent_session_id: parent_id,
            created_at: Self::parse_datetime(&created_at),
            updated_at: Self::parse_datetime(&updated_at),
            last_active_at: Self::parse_datetime(&last_active_at),
            git: git_json.map(|s| serde_json::from_str(&s).unwrap()),
            metrics: serde_json::from_str(&metrics_json).unwrap_or_default(),
            model: serde_json::from_str(&model_json).unwrap_or_default(),
            tags,
            added_files: vec![], // TODO: Load from database when persistence is added
        })
    }

    async fn update_metadata(&self, metadata: &SessionMetadata) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        let git_json = metadata
            .git
            .as_ref()
            .map(|g| serde_json::to_string(g).unwrap());
        let metrics_json = serde_json::to_string(&metadata.metrics)?;
        let model_json = serde_json::to_string(&metadata.model)?;

        conn.execute(
            r#"
            UPDATE sessions SET
                version = ?2, state = ?3, title = ?4,
                updated_at = ?5, last_active_at = ?6,
                git_context = ?7, metrics = ?8, model_config = ?9
            WHERE id = ?1
            "#,
            params![
                metadata.id,
                metadata.version,
                Self::state_to_str(metadata.state),
                metadata.title,
                Self::format_datetime(&metadata.updated_at),
                Self::format_datetime(&metadata.last_active_at),
                git_json,
                metrics_json,
                model_json,
            ],
        )?;

        // Update tags
        conn.execute(
            "DELETE FROM session_tags WHERE session_id = ?1",
            params![metadata.id],
        )?;

        for tag in &metadata.tags {
            conn.execute(
                "INSERT INTO session_tags (session_id, tag) VALUES (?1, ?2)",
                params![metadata.id, tag],
            )?;
        }

        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Delete from database (cascade deletes tags and message_index)
        let rows = conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;

        if rows == 0 {
            return Err(StoreError::NotFound(id.to_string()));
        }

        drop(conn);

        // Delete session directory
        let session_dir = self.sessions_dir.join(id);
        if session_dir.exists() {
            fs::remove_dir_all(session_dir)?;
        }

        Ok(())
    }

    async fn list(&self, query: &SessionQuery) -> Result<Vec<SessionSummary>> {
        let conn = self.conn.lock().unwrap();

        let mut sql = String::from(
            r#"
            SELECT s.id, s.title, s.state, s.working_directory,
                   s.created_at, s.last_active_at, s.message_count, s.metrics
            FROM sessions s
            "#,
        );

        let mut conditions = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref state) = query.state {
            conditions.push(format!("s.state = ?{}", params_vec.len() + 1));
            params_vec.push(Box::new(Self::state_to_str(*state).to_string()));
        }

        if let Some(ref dir) = query.working_directory {
            conditions.push(format!("s.working_directory = ?{}", params_vec.len() + 1));
            params_vec.push(Box::new(dir.to_string_lossy().to_string()));
        }

        if let Some(ref tag) = query.tag {
            sql.push_str(" JOIN session_tags st ON s.id = st.session_id");
            conditions.push(format!("st.tag = ?{}", params_vec.len() + 1));
            params_vec.push(Box::new(tag.clone()));
        }

        if let Some(ref search) = query.search {
            sql.push_str(" JOIN sessions_fts fts ON s.id = fts.session_id");
            conditions.push(format!("sessions_fts MATCH ?{}", params_vec.len() + 1));
            params_vec.push(Box::new(search.clone()));
        }

        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }

        sql.push_str(" ORDER BY s.last_active_at DESC");
        sql.push_str(&format!(" LIMIT {} OFFSET {}", query.limit, query.offset));

        let params_refs: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_refs.as_slice(), |row| {
            let id: String = row.get(0)?;
            let metrics_json: String = row.get(7)?;
            let metrics: SessionMetrics = serde_json::from_str(&metrics_json).unwrap_or_default();

            Ok(SessionSummary {
                id,
                title: row.get(1)?,
                state: Self::str_to_state(&row.get::<_, String>(2)?),
                working_directory: PathBuf::from(row.get::<_, String>(3)?),
                created_at: Self::parse_datetime(&row.get::<_, String>(4)?),
                last_active_at: Self::parse_datetime(&row.get::<_, String>(5)?),
                message_count: row.get::<_, i64>(6)? as u32,
                total_tokens: metrics.total_tokens_in + metrics.total_tokens_out,
                tags: vec![], // Tags populated separately if needed
            })
        })?;

        let summaries: Vec<SessionSummary> = rows.filter_map(|r| r.ok()).collect();
        Ok(summaries)
    }

    async fn get_most_recent(&self) -> Result<Option<SessionSummary>> {
        let query = SessionQuery::new().with_limit(1);
        let sessions = self.list(&query).await?;
        Ok(sessions.into_iter().next())
    }

    async fn append_message(&self, session_id: &str, message: &Message) -> Result<AppendResult> {
        let message_file = self.message_file_path(session_id);

        // Ensure directory exists
        self.ensure_session_dir(session_id)?;

        // Append to JSONL file
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&message_file)?;

        let offset = file.seek(SeekFrom::End(0))? as i64;
        let json = serde_json::to_string(message)?;
        let byte_length = json.len();

        writeln!(file, "{}", json)?;
        file.flush()?;

        // Record in index
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
            INSERT INTO message_index (
                session_id, message_id, role, timestamp,
                file_offset, byte_length, token_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                session_id,
                message.id,
                format!("{:?}", message.role).to_lowercase(),
                Self::format_datetime(&message.timestamp),
                offset,
                byte_length as i64,
                message.token_count,
            ],
        )?;

        // Update message count
        conn.execute(
            "UPDATE sessions SET message_count = message_count + 1, updated_at = ?2, last_active_at = ?2 WHERE id = ?1",
            params![session_id, Self::format_datetime(&Utc::now())],
        )?;

        let message_index = conn.last_insert_rowid();

        Ok(AppendResult {
            file_offset: offset,
            byte_length,
            message_index,
        })
    }

    async fn get_messages(
        &self,
        session_id: &str,
        from: Option<i64>,
        limit: Option<usize>,
    ) -> Result<Vec<Message>> {
        let message_file = self.message_file_path(session_id);

        if !message_file.exists() {
            return Ok(vec![]);
        }

        let file = File::open(&message_file)?;
        let reader = BufReader::new(file);

        let mut messages = Vec::new();
        let skip = from.unwrap_or(0) as usize;
        let max = limit.unwrap_or(usize::MAX);

        for (idx, line) in reader.lines().enumerate() {
            if idx < skip {
                continue;
            }
            if messages.len() >= max {
                break;
            }

            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            let message: Message = serde_json::from_str(&line)?;
            messages.push(message);
        }

        Ok(messages)
    }

    async fn apply_compaction(
        &self,
        session_id: &str,
        summary: &CompactedSummary,
        boundary: usize,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        let summary_json = serde_json::to_string(summary)?;

        conn.execute(
            r#"
            UPDATE sessions SET
                compact_boundary = ?2,
                summary = ?3,
                updated_at = ?4
            WHERE id = ?1
            "#,
            params![
                session_id,
                boundary as i64,
                summary_json,
                Self::format_datetime(&Utc::now()),
            ],
        )?;

        // Update metrics
        conn.execute(
            r#"
            UPDATE sessions SET
                metrics = json_set(metrics, '$.compaction_count',
                    COALESCE(json_extract(metrics, '$.compaction_count'), 0) + 1)
            WHERE id = ?1
            "#,
            params![session_id],
        )?;

        Ok(())
    }

    async fn set_state(&self, id: &str, state: SessionState) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        let rows = conn.execute(
            "UPDATE sessions SET state = ?2, updated_at = ?3 WHERE id = ?1",
            params![
                id,
                Self::state_to_str(state),
                Self::format_datetime(&Utc::now())
            ],
        )?;

        if rows == 0 {
            return Err(StoreError::NotFound(id.to_string()));
        }

        Ok(())
    }

    async fn touch(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Self::format_datetime(&Utc::now());

        let rows = conn.execute(
            "UPDATE sessions SET last_active_at = ?2, updated_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;

        if rows == 0 {
            return Err(StoreError::NotFound(id.to_string()));
        }

        Ok(())
    }

    async fn find_by_prefix(&self, prefix: &str) -> Result<Vec<SessionSummary>> {
        let conn = self.conn.lock().unwrap();

        let pattern = format!("{}%", prefix);
        let mut stmt = conn.prepare(
            r#"
            SELECT id, title, state, working_directory,
                   created_at, last_active_at, message_count, metrics
            FROM sessions
            WHERE id LIKE ?1
            ORDER BY last_active_at DESC
            LIMIT 10
            "#,
        )?;

        let rows = stmt.query_map(params![pattern], |row| {
            let metrics_json: String = row.get(7)?;
            let metrics: SessionMetrics = serde_json::from_str(&metrics_json).unwrap_or_default();

            Ok(SessionSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                state: Self::str_to_state(&row.get::<_, String>(2)?),
                working_directory: PathBuf::from(row.get::<_, String>(3)?),
                created_at: Self::parse_datetime(&row.get::<_, String>(4)?),
                last_active_at: Self::parse_datetime(&row.get::<_, String>(5)?),
                message_count: row.get::<_, i64>(6)? as u32,
                total_tokens: metrics.total_tokens_in + metrics.total_tokens_out,
                tags: vec![],
            })
        })?;

        let summaries: Vec<SessionSummary> = rows.filter_map(|r| r.ok()).collect();
        Ok(summaries)
    }

    async fn get_aggregated_stats(&self, since: Option<DateTime<Utc>>) -> Result<AggregatedStats> {
        let conn = self.conn.lock().unwrap();

        let sql = r#"
            SELECT
                COUNT(*) as session_count,
                COALESCE(SUM(json_extract(metrics, '$.total_tokens_in')), 0) as tokens_in,
                COALESCE(SUM(json_extract(metrics, '$.total_tokens_out')), 0) as tokens_out,
                COALESCE(SUM(json_extract(metrics, '$.total_cost')), 0.0) as cost,
                COALESCE(SUM(json_extract(metrics, '$.turn_count')), 0) as turns,
                COALESCE(SUM(json_extract(metrics, '$.tool_calls')), 0) as tool_calls
            FROM sessions
            WHERE (?1 IS NULL OR last_active_at >= ?1)
        "#;

        let since_str = since.map(|dt| Self::format_datetime(&dt));

        let stats = conn.query_row(sql, params![since_str], |row| {
            Ok(AggregatedStats {
                session_count: row.get(0)?,
                total_tokens_in: row.get(1)?,
                total_tokens_out: row.get(2)?,
                total_cost: row.get(3)?,
                total_turns: row.get(4)?,
                total_tool_calls: row.get(5)?,
            })
        })?;

        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_store() -> (SqliteSessionStore, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let store = SqliteSessionStore::new(temp_dir.path()).unwrap();
        (store, temp_dir)
    }

    #[tokio::test]
    async fn test_create_and_get_session() {
        let (store, _tmp) = create_test_store();

        let session = Session::new(PathBuf::from("/tmp/test"));
        store.create(&session).await.unwrap();

        let retrieved = store.get(session.id()).await.unwrap();
        assert_eq!(retrieved.id(), session.id());
        assert_eq!(
            retrieved.metadata.working_directory,
            session.metadata.working_directory
        );
    }

    #[tokio::test]
    async fn test_append_and_get_messages() {
        let (store, _tmp) = create_test_store();

        let session = Session::new(PathBuf::from("/tmp/test"));
        store.create(&session).await.unwrap();

        let msg1 = Message::user("Hello");
        let msg2 = Message::assistant("Hi there!");

        store.append_message(session.id(), &msg1).await.unwrap();
        store.append_message(session.id(), &msg2).await.unwrap();

        let messages = store.get_messages(session.id(), None, None).await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content.as_text(), "Hello");
        assert_eq!(messages[1].content.as_text(), "Hi there!");
    }

    #[tokio::test]
    async fn test_list_sessions() {
        let (store, _tmp) = create_test_store();

        let session1 = Session::new(PathBuf::from("/tmp/test1"));
        let session2 = Session::new(PathBuf::from("/tmp/test2"));

        store.create(&session1).await.unwrap();
        store.create(&session2).await.unwrap();

        let query = SessionQuery::new();
        let sessions = store.list(&query).await.unwrap();
        assert_eq!(sessions.len(), 2);
    }

    #[tokio::test]
    async fn test_delete_session() {
        let (store, _tmp) = create_test_store();

        let session = Session::new(PathBuf::from("/tmp/test"));
        store.create(&session).await.unwrap();

        store.delete(session.id()).await.unwrap();

        let result = store.get(session.id()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_find_by_prefix() {
        let (store, _tmp) = create_test_store();

        let session = Session::new(PathBuf::from("/tmp/test"));
        let prefix = &session.id()[..8];

        store.create(&session).await.unwrap();

        let found = store.find_by_prefix(prefix).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, session.id());
    }
}
