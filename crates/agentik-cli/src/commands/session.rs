//! Session management commands.

use chrono::{DateTime, Local, Utc};

use agentik_core::SessionState;
use agentik_session::{
    recovery::{RecoveryError, SessionRecovery},
    store::{SessionSummary, SqliteSessionStore},
};

use crate::{SessionAction, TagAction};

/// Format a datetime for display.
fn format_time(dt: &DateTime<Utc>) -> String {
    let local: DateTime<Local> = dt.with_timezone(&Local);
    local.format("%Y-%m-%d %H:%M").to_string()
}

/// Format a session state for display.
fn format_state(state: SessionState) -> &'static str {
    match state {
        SessionState::Active => "active",
        SessionState::Compacting => "compacting",
        SessionState::Suspended => "suspended",
        SessionState::Sleeping => "sleeping",
        SessionState::Archived => "archived",
    }
}

/// Format a session summary for display.
fn format_session_summary(s: &SessionSummary, verbose: bool) -> String {
    let title = s.title.as_deref().unwrap_or("(untitled)");
    let short_id = &s.id[..8];
    let state = format_state(s.state);
    let time = format_time(&s.last_active_at);

    if verbose {
        format!(
            "{} {} [{}] {} ({} msgs, {} tokens)\n    {}",
            short_id,
            title,
            state,
            time,
            s.message_count,
            s.total_tokens,
            s.working_directory.display()
        )
    } else {
        format!("{} {} [{}] {}", short_id, title, state, time)
    }
}

pub async fn handle(action: SessionAction) -> anyhow::Result<()> {
    // Open the session store
    let store = match SqliteSessionStore::open_default() {
        Ok(store) => store,
        Err(e) => {
            println!("Failed to open session store: {}", e);
            println!("The session database may not exist yet. Start a conversation first.");
            return Ok(());
        }
    };

    let recovery = SessionRecovery::new(store);

    match action {
        SessionAction::List { limit, filter } => {
            list_sessions(&recovery, limit, filter.as_deref()).await?;
        }
        SessionAction::Show { id } => {
            show_session(&recovery, &id).await?;
        }
        SessionAction::Export { id, format } => {
            export_session(&recovery, &id, &format).await?;
        }
        SessionAction::Delete { id } => {
            delete_session(&recovery, &id).await?;
        }
        SessionAction::Title { id, title } => {
            handle_title(&recovery, &id, title.as_deref()).await?;
        }
        SessionAction::Tag { id, action } => {
            handle_tag(&recovery, &id, action).await?;
        }
    }

    Ok(())
}

async fn list_sessions<S: agentik_session::store::SessionStore>(
    recovery: &SessionRecovery<S>,
    limit: usize,
    filter: Option<&str>,
) -> anyhow::Result<()> {
    let sessions = if let Some(state_filter) = filter {
        let state = match state_filter.to_lowercase().as_str() {
            "active" => SessionState::Active,
            "suspended" => SessionState::Suspended,
            "sleeping" => SessionState::Sleeping,
            "archived" => SessionState::Archived,
            "compacting" => SessionState::Compacting,
            _ => {
                println!("Unknown state: {}. Valid states: active, suspended, sleeping, archived, compacting", state_filter);
                return Ok(());
            }
        };
        recovery.list_by_state(state, limit).await?
    } else {
        recovery.list_recent(limit).await?
    };

    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    println!("Sessions ({}):", sessions.len());
    println!();

    for session in &sessions {
        println!("{}", format_session_summary(session, true));
    }

    println!();
    println!("Use 'agentik session show <id>' for details");
    println!("Use 'agentik -c' to continue the most recent session");
    println!("Use 'agentik -r <id>' to resume a specific session");

    Ok(())
}

async fn show_session<S: agentik_session::store::SessionStore>(
    recovery: &SessionRecovery<S>,
    id: &str,
) -> anyhow::Result<()> {
    // Try to find session by prefix
    let session = match recovery.store().find_by_prefix(id).await {
        Ok(matches) if matches.len() == 1 => recovery.store().get(&matches[0].id).await?,
        Ok(matches) if matches.is_empty() => {
            println!("Session not found: {}", id);
            return Ok(());
        }
        Ok(matches) => {
            println!("Ambiguous ID '{}' matches {} sessions:", id, matches.len());
            for m in &matches {
                println!("  {}", format_session_summary(m, false));
            }
            return Ok(());
        }
        Err(e) => {
            println!("Error finding session: {}", e);
            return Ok(());
        }
    };

    let meta = &session.metadata;

    println!("Session: {}", meta.id);
    println!("================================================================================");
    println!();
    println!(
        "Title:       {}",
        meta.title.as_deref().unwrap_or("(untitled)")
    );
    println!("State:       {}", format_state(meta.state));
    println!("Directory:   {}", meta.working_directory.display());
    println!();
    println!("Created:     {}", format_time(&meta.created_at));
    println!("Updated:     {}", format_time(&meta.updated_at));
    println!("Last Active: {}", format_time(&meta.last_active_at));
    println!();
    println!("Messages:    {}", session.messages.len());
    println!("Compacted:   {}", session.compact_boundary);

    if let Some(ref summary) = session.summary {
        println!();
        println!("Summary:");
        println!("  {} messages compacted", summary.messages_compacted);
        println!("  {} files modified", summary.modified_files.len());
        println!("  {} key decisions", summary.key_decisions.len());
    }

    println!();
    println!("Metrics:");
    println!("  Tokens In:  {}", meta.metrics.total_tokens_in);
    println!("  Tokens Out: {}", meta.metrics.total_tokens_out);
    println!("  Cost:       ${:.4}", meta.metrics.total_cost);
    println!("  Tool Calls: {}", meta.metrics.tool_calls);

    println!();
    println!("Model:");
    println!("  Provider:    {}", meta.model.provider);
    println!("  Model:       {}", meta.model.model_id);
    println!("  Temperature: {}", meta.model.temperature);

    if !meta.tags.is_empty() {
        println!();
        println!("Tags: {}", meta.tags.join(", "));
    }

    if let Some(ref git) = meta.git {
        println!();
        println!("Git Context:");
        println!("  Repository: {}", git.repository.display());
        println!("  Branch:     {}", git.branch);
        println!("  Start:      {}", &git.commit_at_start[..8]);
        if let Some(ref end) = git.commit_at_end {
            println!("  End:        {}", &end[..8]);
        }
    }

    // Show recent messages
    if !session.messages.is_empty() {
        println!();
        println!("Recent Messages:");
        println!("----------------");

        let start = if session.messages.len() > 5 {
            session.messages.len() - 5
        } else {
            0
        };

        for (idx, msg) in session.messages[start..].iter().enumerate() {
            let role = format!("{:?}", msg.role);
            let content = msg.content.as_text();
            let preview = if content.len() > 80 {
                format!("{}...", &content[..77])
            } else {
                content
            };
            println!("[{}] {}: {}", start + idx, role, preview);
        }
    }

    println!();
    println!("To resume: agentik -r {}", &meta.id[..8]);

    Ok(())
}

async fn export_session<S: agentik_session::store::SessionStore>(
    recovery: &SessionRecovery<S>,
    id: &str,
    format: &str,
) -> anyhow::Result<()> {
    // Try to find session by prefix
    let session = match recovery.store().find_by_prefix(id).await {
        Ok(matches) if matches.len() == 1 => recovery.store().get(&matches[0].id).await?,
        Ok(matches) if matches.is_empty() => {
            println!("Session not found: {}", id);
            return Ok(());
        }
        Ok(matches) => {
            println!("Ambiguous ID '{}' matches {} sessions", id, matches.len());
            return Ok(());
        }
        Err(e) => {
            println!("Error finding session: {}", e);
            return Ok(());
        }
    };

    match format {
        "json" => {
            let json = serde_json::to_string_pretty(&session)?;
            println!("{}", json);
        }
        "markdown" => {
            let meta = &session.metadata;
            println!("# Session: {}", meta.title.as_deref().unwrap_or("Untitled"));
            println!();
            println!("- **ID:** `{}`", meta.id);
            println!("- **Created:** {}", format_time(&meta.created_at));
            println!("- **Directory:** `{}`", meta.working_directory.display());
            println!();

            if let Some(ref summary) = session.summary {
                println!("## Summary");
                println!();
                println!("{}", summary.text);
                println!();

                if !summary.key_decisions.is_empty() {
                    println!("### Key Decisions");
                    for d in &summary.key_decisions {
                        println!("- {}", d);
                    }
                    println!();
                }

                if !summary.modified_files.is_empty() {
                    println!("### Modified Files");
                    for f in &summary.modified_files {
                        println!("- `{}`", f.display());
                    }
                    println!();
                }
            }

            println!("## Conversation");
            println!();

            for msg in &session.messages {
                let role = match msg.role {
                    agentik_core::Role::User => "**User**",
                    agentik_core::Role::Assistant => "**Assistant**",
                    agentik_core::Role::System => "**System**",
                    agentik_core::Role::Tool => "**Tool**",
                };
                println!("{}", role);
                println!();
                println!("{}", msg.content.as_text());
                println!();
                println!("---");
                println!();
            }
        }
        _ => {
            println!("Unknown format: {}. Supported: json, markdown", format);
        }
    }

    Ok(())
}

async fn delete_session<S: agentik_session::store::SessionStore>(
    recovery: &SessionRecovery<S>,
    id: &str,
) -> anyhow::Result<()> {
    // Try to find session by prefix
    let matches = recovery.store().find_by_prefix(id).await?;

    match matches.len() {
        0 => {
            println!("Session not found: {}", id);
        }
        1 => {
            let session_id = &matches[0].id;
            recovery.store().delete(session_id).await?;
            println!("Deleted session: {}", session_id);
        }
        n => {
            println!("Ambiguous ID '{}' matches {} sessions:", id, n);
            for m in &matches {
                println!("  {}", format_session_summary(m, false));
            }
            println!();
            println!("Please provide a more specific ID.");
        }
    }

    Ok(())
}

/// Resume a session for the interactive mode.
///
/// This function is called from main.rs when -c or -r flags are used.
#[allow(dead_code)]
pub async fn resume_session(id_or_prefix: Option<&str>) -> anyhow::Result<agentik_core::Session> {
    let store = SqliteSessionStore::open_default()?;
    let recovery = SessionRecovery::new(store);

    match recovery.smart_resume(id_or_prefix).await {
        Ok(session) => {
            let short_id = &session.id()[..8];
            let title = session.metadata.title.as_deref().unwrap_or("(untitled)");
            println!("Resuming session: {} - {}", short_id, title);
            Ok(session)
        }
        Err(RecoveryError::NoSessionsFound) => {
            anyhow::bail!("No sessions found. Start a new conversation first.")
        }
        Err(RecoveryError::SessionNotFound(id)) => {
            anyhow::bail!("Session not found: {}", id)
        }
        Err(RecoveryError::AmbiguousPrefix(prefix, count)) => {
            anyhow::bail!(
                "Ambiguous ID '{}' matches {} sessions. Use a longer prefix.",
                prefix,
                count
            )
        }
        Err(RecoveryError::InvalidState(state)) => {
            anyhow::bail!(
                "Cannot resume session in {:?} state. Use a different session.",
                state
            )
        }
        Err(e) => anyhow::bail!("Failed to resume session: {}", e),
    }
}

/// Handle session title command.
async fn handle_title<S: agentik_session::store::SessionStore>(
    recovery: &SessionRecovery<S>,
    id: &str,
    title: Option<&str>,
) -> anyhow::Result<()> {
    // Try to find session by prefix
    let matches = recovery.store().find_by_prefix(id).await?;

    match matches.len() {
        0 => {
            println!("Session not found: {}", id);
        }
        1 => {
            let session_id = &matches[0].id;
            let mut meta = recovery.store().get_metadata(session_id).await?;

            match title {
                Some(new_title) => {
                    if new_title.len() > 100 {
                        println!("Title too long (max 100 characters)");
                        return Ok(());
                    }
                    meta.title = Some(new_title.to_string());
                    recovery.store().update_metadata(&meta).await?;
                    println!("Title set to: {}", new_title);
                }
                None => {
                    println!(
                        "Title: {}",
                        meta.title.as_deref().unwrap_or("(untitled)")
                    );
                }
            }
        }
        n => {
            println!("Ambiguous ID '{}' matches {} sessions:", id, n);
            for m in &matches {
                println!("  {}", format_session_summary(m, false));
            }
            println!();
            println!("Please provide a more specific ID.");
        }
    }

    Ok(())
}

/// Handle session tag command.
async fn handle_tag<S: agentik_session::store::SessionStore>(
    recovery: &SessionRecovery<S>,
    id: &str,
    action: TagAction,
) -> anyhow::Result<()> {
    // Try to find session by prefix
    let matches = recovery.store().find_by_prefix(id).await?;

    match matches.len() {
        0 => {
            println!("Session not found: {}", id);
        }
        1 => {
            let session_id = &matches[0].id;
            let mut meta = recovery.store().get_metadata(session_id).await?;

            match action {
                TagAction::Add { tag } => {
                    if tag.len() > 50 {
                        println!("Tag too long (max 50 characters)");
                        return Ok(());
                    }
                    if tag.contains(',') {
                        println!("Tags cannot contain commas");
                        return Ok(());
                    }
                    if meta.tags.contains(&tag) {
                        println!("Tag already exists: {}", tag);
                    } else {
                        meta.tags.push(tag.clone());
                        recovery.store().update_metadata(&meta).await?;
                        println!("Tag added: {}", tag);
                    }
                }
                TagAction::Remove { tag } => {
                    if let Some(pos) = meta.tags.iter().position(|t| t == &tag) {
                        meta.tags.remove(pos);
                        recovery.store().update_metadata(&meta).await?;
                        println!("Tag removed: {}", tag);
                    } else {
                        println!("Tag not found: {}", tag);
                    }
                }
                TagAction::List => {
                    if meta.tags.is_empty() {
                        println!("No tags set");
                    } else {
                        println!("Tags: {}", meta.tags.join(", "));
                    }
                }
            }
        }
        n => {
            println!("Ambiguous ID '{}' matches {} sessions:", id, n);
            for m in &matches {
                println!("  {}", format_session_summary(m, false));
            }
            println!();
            println!("Please provide a more specific ID.");
        }
    }

    Ok(())
}
