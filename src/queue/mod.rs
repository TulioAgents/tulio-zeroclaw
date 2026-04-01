//! Persistent per-agent task queue backed by SQLite.
//!
//! Each agent has an inbox of `QueueItem` rows stored at:
//!   `~/.zeroclaw/queue/items.db`
//!
//! Items survive reboots and are drained by a cron `JobType::Agent` job
//! registered automatically for each configured agent.
//!
//! Lifecycle: pending → processing → done | failed
//! FIFO within the same priority (higher priority = lower number = dequeued first).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum QueueStatus {
    Pending,
    Processing,
    Done,
    Failed,
}

impl QueueStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Processing => "processing",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

impl TryFrom<&str> for QueueStatus {
    type Error = String;
    fn try_from(s: &str) -> std::result::Result<Self, Self::Error> {
        match s {
            "pending" => Ok(Self::Pending),
            "processing" => Ok(Self::Processing),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            other => Err(format!("Unknown queue status: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: String,
    pub agent: String,
    pub task: String,
    pub context: Option<String>,
    pub from_agent: Option<String>,
    /// Lower number = higher priority. Default 0.
    pub priority: i32,
    pub status: QueueStatus,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub result: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EnqueueRequest {
    pub task: String,
    pub context: Option<String>,
    pub from_agent: Option<String>,
    pub priority: Option<i32>,
}

// ── DB helpers ───────────────────────────────────────────────────────────────

fn with_connection<T>(
    workspace_dir: &Path,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let db_path = workspace_dir.join("queue").join("items.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create queue dir: {}", parent.display()))?;
    }

    let conn = Connection::open(&db_path)
        .with_context(|| format!("Failed to open queue DB: {}", db_path.display()))?;

    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS queue_items (
             id           TEXT PRIMARY KEY,
             agent        TEXT NOT NULL,
             task         TEXT NOT NULL,
             context      TEXT,
             from_agent   TEXT,
             priority     INTEGER NOT NULL DEFAULT 0,
             status       TEXT NOT NULL DEFAULT 'pending',
             created_at   TEXT NOT NULL,
             started_at   TEXT,
             finished_at  TEXT,
             result       TEXT,
             error        TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_queue_agent_status
             ON queue_items (agent, status, priority ASC, created_at ASC);",
    )
    .context("Failed to initialize queue DB schema")?;

    f(&conn)
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<QueueItem> {
    let status_str: String = row.get(6)?;
    let status = QueueStatus::try_from(status_str.as_str())
        .unwrap_or(QueueStatus::Pending);

    let parse_dt = |s: Option<String>| -> Option<DateTime<Utc>> {
        s.as_deref().and_then(|v| v.parse().ok())
    };

    Ok(QueueItem {
        id: row.get(0)?,
        agent: row.get(1)?,
        task: row.get(2)?,
        context: row.get(3)?,
        from_agent: row.get(4)?,
        priority: row.get(5)?,
        status,
        created_at: row
            .get::<_, String>(7)?
            .parse()
            .unwrap_or_else(|_| Utc::now()),
        started_at: parse_dt(row.get(8)?),
        finished_at: parse_dt(row.get(9)?),
        result: row.get(10)?,
        error: row.get(11)?,
    })
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Add a new task to an agent's queue. Returns the created item.
pub fn enqueue(workspace_dir: &Path, agent: &str, req: EnqueueRequest) -> Result<QueueItem> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let priority = req.priority.unwrap_or(0);

    with_connection(workspace_dir, |conn| {
        conn.execute(
            "INSERT INTO queue_items (id, agent, task, context, from_agent, priority, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7)",
            params![id, agent, req.task, req.context, req.from_agent, priority, now],
        )
        .context("Failed to insert queue item")?;

        let mut stmt = conn.prepare(
            "SELECT id, agent, task, context, from_agent, priority, status,
                    created_at, started_at, finished_at, result, error
             FROM queue_items WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            map_row(row).map_err(Into::into)
        } else {
            anyhow::bail!("Queue item '{id}' not found after insert")
        }
    })
}

/// Fetch the next pending item for an agent (highest priority, then FIFO).
/// Does NOT change status — call `mark_processing` after claiming it.
pub fn peek_next(workspace_dir: &Path, agent: &str) -> Result<Option<QueueItem>> {
    with_connection(workspace_dir, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, agent, task, context, from_agent, priority, status,
                    created_at, started_at, finished_at, result, error
             FROM queue_items
             WHERE agent = ?1 AND status = 'pending'
             ORDER BY priority ASC, created_at ASC
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![agent])?;
        if let Some(row) = rows.next()? {
            Ok(Some(map_row(row)?))
        } else {
            Ok(None)
        }
    })
}

/// Mark an item as processing (claimed by the drain job).
pub fn mark_processing(workspace_dir: &Path, id: &str) -> Result<()> {
    with_connection(workspace_dir, |conn| {
        conn.execute(
            "UPDATE queue_items SET status = 'processing', started_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id],
        )
        .context("Failed to mark item processing")?;
        Ok(())
    })
}

/// Mark a processing item as done with its result.
pub fn mark_done(workspace_dir: &Path, id: &str, result: &str) -> Result<()> {
    with_connection(workspace_dir, |conn| {
        conn.execute(
            "UPDATE queue_items SET status = 'done', finished_at = ?1, result = ?2 WHERE id = ?3",
            params![Utc::now().to_rfc3339(), result, id],
        )
        .context("Failed to mark item done")?;
        Ok(())
    })
}

/// Mark a processing item as failed with an error message.
pub fn mark_failed(workspace_dir: &Path, id: &str, error: &str) -> Result<()> {
    with_connection(workspace_dir, |conn| {
        conn.execute(
            "UPDATE queue_items SET status = 'failed', finished_at = ?1, error = ?2 WHERE id = ?3",
            params![Utc::now().to_rfc3339(), error, id],
        )
        .context("Failed to mark item failed")?;
        Ok(())
    })
}

/// List items for an agent, optionally filtered by status.
pub fn list(
    workspace_dir: &Path,
    agent: &str,
    status: Option<&str>,
) -> Result<Vec<QueueItem>> {
    with_connection(workspace_dir, |conn| {
        let items = if let Some(s) = status {
            let mut stmt = conn.prepare(
                "SELECT id, agent, task, context, from_agent, priority, status,
                        created_at, started_at, finished_at, result, error
                 FROM queue_items
                 WHERE agent = ?1 AND status = ?2
                 ORDER BY priority ASC, created_at ASC",
            )?;
            let rows = stmt.query_map(params![agent, s], map_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, agent, task, context, from_agent, priority, status,
                        created_at, started_at, finished_at, result, error
                 FROM queue_items
                 WHERE agent = ?1
                 ORDER BY priority ASC, created_at ASC",
            )?;
            let rows = stmt.query_map(params![agent], map_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(items)
    })
}

/// Count pending items for an agent.
pub fn pending_count(workspace_dir: &Path, agent: &str) -> Result<u64> {
    with_connection(workspace_dir, |conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM queue_items WHERE agent = ?1 AND status = 'pending'",
            params![agent],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u64)
    })
}

/// Cancel (delete) a pending item by id. Returns error if not found or not pending.
pub fn cancel(workspace_dir: &Path, id: &str) -> Result<()> {
    with_connection(workspace_dir, |conn| {
        let changed = conn
            .execute(
                "DELETE FROM queue_items WHERE id = ?1 AND status = 'pending'",
                params![id],
            )
            .context("Failed to cancel queue item")?;
        if changed == 0 {
            anyhow::bail!("Queue item '{id}' not found or not in pending state");
        }
        Ok(())
    })
}

/// Return all queue items whose context contains `project: <project>`.
/// If `task_id` is Some, further filters to items whose context also contains
/// `task_id: <task_id>` as an exact line.
/// Ordered by created_at DESC (newest attempt first).
pub fn query_by_project(
    workspace_dir: &Path,
    project: &str,
    task_id: Option<&str>,
) -> Result<Vec<QueueItem>> {
    let project_needle = format!("project: {project}");
    with_connection(workspace_dir, |conn| {
        let items = if let Some(tid) = task_id {
            let task_needle = format!("task_id: {tid}");
            let mut stmt = conn.prepare(
                "SELECT id, agent, task, context, from_agent, priority, status,
                        created_at, started_at, finished_at, result, error
                 FROM queue_items
                 WHERE instr(coalesce(context,''), ?1) > 0
                   AND instr(coalesce(context,''), ?2) > 0
                 ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map(params![project_needle, task_needle], map_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, agent, task, context, from_agent, priority, status,
                        created_at, started_at, finished_at, result, error
                 FROM queue_items
                 WHERE instr(coalesce(context,''), ?1) > 0
                 ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map(params![project_needle], map_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(items)
    })
}

// ── CLI handler ──────────────────────────────────────────────────────────────

pub async fn handle_command(
    cmd: crate::QueueCommands,
    config: &crate::config::Config,
) -> anyhow::Result<()> {
    let workspace = &config.workspace_dir;

    match cmd {
        crate::QueueCommands::List { agent, status } => {
            let items = list(workspace, &agent, status.as_deref())?;
            if items.is_empty() {
                println!("No queue items for agent '{agent}'.");
            } else {
                println!("Queue for '{agent}' ({} items):", items.len());
                for item in &items {
                    let age = chrono::Utc::now()
                        .signed_duration_since(item.created_at)
                        .num_seconds();
                    println!(
                        "  [{:?}] {} — {} ({}s ago, from: {})",
                        item.status,
                        item.id,
                        &item.task[..item.task.len().min(60)],
                        age,
                        item.from_agent.as_deref().unwrap_or("—"),
                    );
                }
            }
            Ok(())
        }

        crate::QueueCommands::Drain { agent, limit } => {
            // Reset any stale items from a previous crashed run
            let reset = reset_stale(workspace, 10)?;
            if reset > 0 {
                println!("↩️  Reset {reset} stale processing items.");
            }

            let mut drained = 0usize;
            for _ in 0..limit {
                let Some(item) = peek_next(workspace, &agent)? else {
                    break;
                };
                println!("▶  Processing [{agent}] {}: {:.60}", item.id, item.task);

                mark_processing(workspace, &item.id)?;
                crate::health::mark_agent_active(&agent, &item.id);

                let prompt = build_drain_prompt(&agent, &item);

                // Point workspace_dir at the agent's own identity directory
                // so run() loads IDENTITY.md + AGENTS.md for this specific agent.
                let mut agent_config = config.clone();
                let agent_workspace = config.workspace_dir.join("agents").join(&agent);
                if agent_workspace.is_dir() {
                    agent_config.workspace_dir = agent_workspace;
                }

                let result = crate::agent::run(
                    agent_config,
                    Some(prompt),
                    None,
                    None,
                    config.default_temperature,
                    vec![],
                    false,
                )
                .await;

                crate::health::mark_agent_idle(&agent);

                match result {
                    Ok(response) => {
                        mark_done(workspace, &item.id, &response)?;
                        println!("✅ Done [{agent}] {}", item.id);
                    }
                    Err(e) => {
                        let err = e.to_string();
                        mark_failed(workspace, &item.id, &err)?;
                        println!("❌ Failed [{agent}] {}: {err}", item.id);
                    }
                }
                drained += 1;
            }

            if drained == 0 {
                println!("Queue empty for '{agent}' — nothing to drain.");
            } else {
                println!("Drained {drained} item(s) for '{agent}'.");
            }
            Ok(())
        }

        crate::QueueCommands::Cancel { agent: _, id } => {
            cancel(workspace, &id)?;
            println!("✅ Cancelled queue item {id}.");
            Ok(())
        }

        crate::QueueCommands::Reset { agent, stale_minutes } => {
            let agents_to_reset: Vec<String> = if agent == "all" {
                // Collect distinct agents from the DB
                with_connection(workspace, |conn| {
                    let mut stmt = conn.prepare(
                        "SELECT DISTINCT agent FROM queue_items WHERE status = 'processing'",
                    )?;
                    let names = stmt
                        .query_map([], |row| row.get::<_, String>(0))?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok(names)
                })?
            } else {
                vec![agent]
            };

            let mut total = 0u64;
            for a in &agents_to_reset {
                total += reset_stale(workspace, stale_minutes)?;
                println!("↩️  Reset stale items for '{a}'.");
            }
            println!("Total reset: {total}.");
            Ok(())
        }
    }
}

fn build_drain_prompt(agent: &str, item: &QueueItem) -> String {
    let mut prompt = format!(
        "[queue-drain:{agent} item:{}]\n\nYour task:\n{}\n",
        item.id, item.task
    );
    if let Some(ctx) = &item.context {
        prompt.push_str("\nContext:\n");
        prompt.push_str(ctx);
        prompt.push('\n');

        // Extract task_id and project from the context block.
        // Convention: `task_id: hw3-003` and `project: helloworld3` as bare lines.
        let task_id_in_ctx = ctx
            .lines()
            .find_map(|l| l.trim().strip_prefix("task_id: "))
            .map(str::trim);
        let project_in_ctx = ctx
            .lines()
            .find_map(|l| l.trim().strip_prefix("project: "))
            .map(str::trim);

        // Inject a TASK GUARD when both are present so the agent knows:
        // 1) which tasks.md row it owns
        // 2) to use its OWN task_id in queue_enqueue, not the queue item id
        if let (Some(tid), Some(proj)) = (task_id_in_ctx, project_in_ctx) {
            prompt.push_str(&format!(
                "\n[TASK GUARD]\n\
                 You are responsible for task `{tid}` in project `{proj}`.\n\
                 - In tasks.md, update ONLY the row with ID `{tid}` (set Status to \
                   `in-progress` when you start, `done` when you finish).\n\
                 - When calling queue_enqueue to hand off to the next agent, set \
                   `task_id` to the NEXT agent's task row ID from tasks.md — NOT \
                   this queue item id `{}`.\n\
                 - tasks.md is at: \
                   ~/coding-projects/{proj}/openspec/changes/<change-id>/tasks.md\n\
                 - Read tasks.md first to confirm your task is not already `done`. \
                   If it is, output: SKIP: task {tid} already done — and stop.\n",
                item.id
            ));
        }
    }
    if let Some(from) = &item.from_agent {
        prompt.push_str(&format!("\nRequested by: {from}\n"));
    }
    prompt.push_str("\nComplete the task above. Be thorough but concise in your response.");
    prompt
}

/// Reset stale `processing` items back to `pending` (e.g. after reboot).
/// Items stuck in processing for more than `stale_minutes` are reset.
pub fn reset_stale(workspace_dir: &Path, stale_minutes: i64) -> Result<u64> {
    let cutoff = Utc::now() - chrono::Duration::minutes(stale_minutes);
    with_connection(workspace_dir, |conn| {
        let changed = conn
            .execute(
                "UPDATE queue_items SET status = 'pending', started_at = NULL
                 WHERE status = 'processing' AND started_at < ?1",
                params![cutoff.to_rfc3339()],
            )
            .context("Failed to reset stale queue items")?;
        Ok(changed as u64)
    })
}
