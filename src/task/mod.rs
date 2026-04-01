//! Task tracking: parse tasks.md, cross-reference queue DB, show/fix drift.
//!
//! Usage:
//!   zeroclaw task status --project helloworld3 --change hw3-001
//!   zeroclaw task status --project helloworld3 --change hw3-001 --sync

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct TaskRow {
    id: String,
    title: String,
    owner: String,
    md_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum QueueVerdict {
    /// No queue item found for this task_id
    NotQueued,
    /// At least one item is pending or processing
    InProgress,
    /// All matching items are done
    Done,
    /// At least one item failed (and none are still in-progress)
    Failed,
}

// ── Home dir ─────────────────────────────────────────────────────────────────

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

// ── tasks.md path ─────────────────────────────────────────────────────────────

fn tasks_md_path(project: &str, change: &str) -> Result<PathBuf> {
    let home = home_dir().context("Cannot resolve $HOME")?;
    Ok(home
        .join("coding-projects")
        .join(project)
        .join("openspec")
        .join("changes")
        .join(change)
        .join("tasks.md"))
}

// ── tasks.md parsing ─────────────────────────────────────────────────────────

fn parse_tasks_md(content: &str) -> Vec<TaskRow> {
    let mut rows = Vec::new();
    let mut in_table = false;
    let mut past_separator = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') {
            // A non-pipe line after the table started means we're done.
            if in_table {
                break;
            }
            continue;
        }

        if !in_table {
            // Detect header row by case-insensitive column name check.
            let lower = trimmed.to_ascii_lowercase();
            if lower.contains("| id") || lower.starts_with("|id") {
                in_table = true;
                past_separator = false;
            }
            // Either way don't parse this as a data row.
            continue;
        }

        // First row after the header: separator (|---|) — skip it.
        if !past_separator {
            past_separator = true;
            if trimmed.contains("---") {
                continue;
            }
            // No separator row — fall through to parse as data.
        }

        let cells: Vec<&str> = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();

        if cells.len() < 4 || cells[0].is_empty() {
            continue;
        }

        rows.push(TaskRow {
            id: cells[0].to_string(),
            title: cells[1].to_string(),
            owner: cells[2].to_string(),
            md_status: cells[3].to_string(),
        });
    }
    rows
}

// ── tasks.md patching ────────────────────────────────────────────────────────

/// Rewrite tasks.md content, replacing only the Status cell for rows in `patches`.
/// `patches` is a slice of (task_id, new_status) pairs.
fn patch_tasks_md(content: &str, patches: &[(String, String)]) -> String {
    let mut output = String::with_capacity(content.len() + 64);
    let mut in_table = false;
    let mut past_separator = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('|') {
            if !in_table {
                let lower = trimmed.to_ascii_lowercase();
                if lower.contains("| id") || lower.starts_with("|id") {
                    in_table = true;
                    past_separator = false;
                }
                output.push_str(line);
                output.push('\n');
                continue;
            }

            if !past_separator {
                past_separator = true;
                output.push_str(line); // separator verbatim
                output.push('\n');
                continue;
            }

            // Data row — attempt patch.
            let cells: Vec<&str> = trimmed
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect();

            if cells.len() >= 4 {
                if let Some((_, new_status)) =
                    patches.iter().find(|(id, _)| id == cells[0])
                {
                    let status_width = cells[3].len().max(new_status.len());
                    output.push_str(&format!(
                        "| {:<w0$} | {:<w1$} | {:<w2$} | {:<w3$} |",
                        cells[0],
                        cells[1],
                        cells[2],
                        new_status,
                        w0 = cells[0].len(),
                        w1 = cells[1].len(),
                        w2 = cells[2].len(),
                        w3 = status_width,
                    ));
                    output.push('\n');
                    continue;
                }
            }
        } else if in_table {
            in_table = false;
        }

        output.push_str(line);
        output.push('\n');
    }
    output
}

// ── Queue cross-reference ────────────────────────────────────────────────────

fn verdict_for_task(
    items: &[crate::queue::QueueItem],
    task_id: &str,
) -> (QueueVerdict, usize) {
    // Match items whose context has `task_id: <id>` as an exact trimmed line.
    let needle = format!("task_id: {task_id}");
    let matching: Vec<_> = items
        .iter()
        .filter(|item| {
            item.context
                .as_deref()
                .map(|c| c.lines().any(|l| l.trim() == needle.as_str()))
                .unwrap_or(false)
        })
        .collect();

    let count = matching.len();
    if count == 0 {
        return (QueueVerdict::NotQueued, 0);
    }

    use crate::queue::QueueStatus;
    if matching
        .iter()
        .any(|i| matches!(i.status, QueueStatus::Pending | QueueStatus::Processing))
    {
        return (QueueVerdict::InProgress, count);
    }
    if matching.iter().any(|i| i.status == QueueStatus::Failed) {
        return (QueueVerdict::Failed, count);
    }
    if matching.iter().all(|i| i.status == QueueStatus::Done) {
        return (QueueVerdict::Done, count);
    }
    (QueueVerdict::NotQueued, count)
}

// ── CLI handler ───────────────────────────────────────────────────────────────

pub async fn handle_command(
    cmd: crate::TaskCommands,
    config: &crate::config::Config,
) -> Result<()> {
    match cmd {
        crate::TaskCommands::Status {
            project,
            change,
            sync,
        } => handle_status(&project, &change, sync, &config.workspace_dir).await,
    }
}

async fn handle_status(
    project: &str,
    change: &str,
    sync: bool,
    workspace_dir: &Path,
) -> Result<()> {
    let path = tasks_md_path(project, change)?;
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Cannot read tasks.md at {}", path.display()))?;

    let tasks = parse_tasks_md(&content);
    if tasks.is_empty() {
        println!("No tasks found in {}", path.display());
        return Ok(());
    }

    // One DB round-trip for all queue items belonging to this project.
    let queue_items =
        crate::queue::query_by_project(workspace_dir, project, None)?;

    println!();
    println!("  Project : {project}");
    println!("  Change  : {change}");
    println!("  File    : {}", path.display());
    println!("  Queue   : {} item(s) for this project", queue_items.len());
    println!();
    println!(
        "  {:<12}  {:<32}  {:<16}  {:<12}  {:<14}  {}",
        "Task ID", "Title", "Owner", "File status", "Queue status", "Drift?"
    );
    println!("  {}", "─".repeat(100));

    let mut patches: Vec<(String, String)> = Vec::new();
    let mut drift_count = 0usize;

    for task in &tasks {
        let (verdict, q_count) = verdict_for_task(&queue_items, &task.id);

        let md_lower = task.md_status.to_ascii_lowercase();
        let drift = !matches!(
            (md_lower.as_str(), &verdict),
            ("done", QueueVerdict::Done)
                | ("in-progress", QueueVerdict::InProgress)
                | ("todo", QueueVerdict::NotQueued)
                | ("failed", QueueVerdict::Failed)
        );

        if drift {
            drift_count += 1;
        }

        let queue_label = match &verdict {
            QueueVerdict::NotQueued => "─".to_string(),
            QueueVerdict::InProgress => format!("in-progress ({q_count})"),
            QueueVerdict::Done => format!("done ({q_count})"),
            QueueVerdict::Failed => format!("FAILED ({q_count})"),
        };

        let drift_marker = if drift { "  <-- DRIFT" } else { "" };

        println!(
            "  {:<12}  {:<32}  {:<16}  {:<12}  {:<14}  {}",
            task.id,
            &task.title[..task.title.len().min(31)],
            &task.owner[..task.owner.len().min(15)],
            task.md_status,
            queue_label,
            drift_marker,
        );

        if sync && drift {
            let new_status = match &verdict {
                QueueVerdict::Done => Some("done"),
                QueueVerdict::InProgress => Some("in-progress"),
                QueueVerdict::Failed => Some("failed"),
                QueueVerdict::NotQueued => None, // nothing to sync to
            };
            if let Some(s) = new_status {
                patches.push((task.id.clone(), s.to_string()));
            }
        }
    }

    println!();

    if drift_count == 0 {
        println!("  No drift detected — tasks.md is consistent with the queue.");
    } else {
        println!("  {drift_count} row(s) with drift.");
    }

    if sync {
        if patches.is_empty() {
            println!("  Nothing to patch.");
        } else {
            println!();
            println!("  Syncing {} row(s) (queue is truth)...", patches.len());
            let updated = patch_tasks_md(&content, &patches);
            std::fs::write(&path, &updated)
                .with_context(|| format!("Cannot write {}", path.display()))?;
            for (id, status) in &patches {
                println!("    {id}  ->  {status}");
            }
            println!("  Saved.");
        }
    } else if drift_count > 0 {
        println!("  Run with --sync to patch tasks.md.");
    }

    println!();
    Ok(())
}
