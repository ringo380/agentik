-- Initial schema for Agentik session storage
-- Version 1

-- Sessions table: core metadata
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    state TEXT NOT NULL DEFAULT 'active',
    working_directory TEXT NOT NULL,
    title TEXT,
    parent_session_id TEXT,

    -- Timestamps
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_active_at TEXT NOT NULL,

    -- Git context (JSON)
    git_context TEXT,

    -- Usage metrics (JSON)
    metrics TEXT NOT NULL DEFAULT '{}',

    -- Model configuration (JSON)
    model_config TEXT NOT NULL DEFAULT '{}',

    -- Compaction state
    compact_boundary INTEGER NOT NULL DEFAULT 0,
    summary TEXT,

    -- Message file info
    message_file TEXT,
    message_count INTEGER NOT NULL DEFAULT 0,

    FOREIGN KEY (parent_session_id) REFERENCES sessions(id)
);

-- Indexes for common queries
CREATE INDEX IF NOT EXISTS idx_sessions_state ON sessions(state);
CREATE INDEX IF NOT EXISTS idx_sessions_working_directory ON sessions(working_directory);
CREATE INDEX IF NOT EXISTS idx_sessions_last_active_at ON sessions(last_active_at DESC);
CREATE INDEX IF NOT EXISTS idx_sessions_created_at ON sessions(created_at DESC);

-- Session tags: many-to-many relationship
CREATE TABLE IF NOT EXISTS session_tags (
    session_id TEXT NOT NULL,
    tag TEXT NOT NULL,
    PRIMARY KEY (session_id, tag),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_session_tags_tag ON session_tags(tag);

-- Message index: quick lookup without parsing JSONL
CREATE TABLE IF NOT EXISTS message_index (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    role TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    file_offset INTEGER NOT NULL,
    byte_length INTEGER NOT NULL,
    token_count INTEGER,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_message_index_session ON message_index(session_id, id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_message_index_message_id ON message_index(message_id);

-- Full-text search on session titles and summaries
CREATE VIRTUAL TABLE IF NOT EXISTS sessions_fts USING fts5(
    session_id,
    title,
    summary,
    content='',
    tokenize='porter'
);

-- Triggers to keep FTS in sync
CREATE TRIGGER IF NOT EXISTS sessions_fts_insert AFTER INSERT ON sessions BEGIN
    INSERT INTO sessions_fts(session_id, title, summary)
    VALUES (NEW.id, COALESCE(NEW.title, ''), COALESCE(NEW.summary, ''));
END;

CREATE TRIGGER IF NOT EXISTS sessions_fts_update AFTER UPDATE ON sessions BEGIN
    DELETE FROM sessions_fts WHERE session_id = OLD.id;
    INSERT INTO sessions_fts(session_id, title, summary)
    VALUES (NEW.id, COALESCE(NEW.title, ''), COALESCE(NEW.summary, ''));
END;

CREATE TRIGGER IF NOT EXISTS sessions_fts_delete AFTER DELETE ON sessions BEGIN
    DELETE FROM sessions_fts WHERE session_id = OLD.id;
END;

-- Schema version tracking
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);

INSERT OR IGNORE INTO schema_version (version, applied_at) VALUES (1, datetime('now'));
