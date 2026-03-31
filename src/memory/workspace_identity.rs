//! Workspace identity hydration — seeds agent identity files into memory at session start.
//!
//! Instead of injecting all workspace `.md` files into the system prompt every session,
//! this module stores each file as a `Core` memory entry once. The existing per-message
//! RAG pipeline then surfaces only the relevant pieces for each user message, saving
//! tokens on every turn.
//!
//! ## File → memory key mapping
//!
//! | File           | Memory key              | Category |
//! |----------------|-------------------------|----------|
//! | `IDENTITY.md`  | `agent:identity`        | Core     |
//! | `SOUL.md`      | `agent:soul`            | Core     |
//! | `AGENTS.md`    | `agent:topology`        | Core     |
//! | `USER.md`      | `agent:user`            | Core     |
//! | `TOOLS.md`     | `agent:tools`           | Core     |
//! | `HEARTBEAT.md` | `agent:heartbeat`       | Daily    |
//! | `BOOTSTRAP.md` | `agent:bootstrap`       | Core     |
//!
//! Entries are written with `store()` only when not already present (idempotent).
//! `BOOTSTRAP.md` is renamed to `BOOTSTRAP.md.done` after first hydration so it
//! only runs once — matching OpenClaw's one-shot design intent.

use crate::memory::traits::{Memory, MemoryCategory};
use std::path::Path;
use tracing::{debug, info, warn};

/// File → (memory key, category_str, one_shot)
/// `one_shot = true` renames the file to `<name>.done` after first hydration.
/// Using `&str` for category avoids the `Copy` constraint on `MemoryCategory`.
const IDENTITY_FILES: &[(&str, &str, &str, bool)] = &[
    ("IDENTITY.md", "agent:identity", "core", false),
    ("SOUL.md", "agent:soul", "core", false),
    ("AGENTS.md", "agent:topology", "core", false),
    ("USER.md", "agent:user", "core", false),
    ("TOOLS.md", "agent:tools", "core", false),
    ("HEARTBEAT.md", "agent:heartbeat", "daily", false),
    ("BOOTSTRAP.md", "agent:bootstrap", "core", true),
];

/// Seed workspace identity files into memory as `Core` entries.
///
/// Skips files that are already stored (idempotent — safe to call every session
/// start; no-op once entries exist). Returns the number of entries written.
pub async fn hydrate_workspace_identity(
    mem: &dyn Memory,
    workspace_dir: &Path,
) -> anyhow::Result<usize> {
    let mut written = 0;

    for (filename, key, category_str, one_shot) in IDENTITY_FILES {
        let category = match *category_str {
            "daily" => MemoryCategory::Daily,
            _ => MemoryCategory::Core,
        };
        let path = workspace_dir.join(filename);

        // Check .done variant first (one-shot files already processed)
        if *one_shot {
            let done_path = workspace_dir.join(format!("{filename}.done"));
            if done_path.exists() {
                debug!("workspace identity: {filename} already processed (found .done), skipping");
                continue;
            }
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => {
                debug!("workspace identity: {filename} not found, skipping");
                continue;
            }
        };

        let trimmed = content.trim();
        if trimmed.is_empty() {
            debug!("workspace identity: {filename} is empty, skipping");
            continue;
        }

        // Skip if already in memory — idempotent
        if let Ok(Some(_)) = mem.get(key).await {
            debug!("workspace identity: {key} already in memory, skipping");
            continue;
        }

        match mem.store(key, trimmed, category, None).await {
            Ok(()) => {
                info!("workspace identity: stored {key} from {filename}");
                written += 1;

                // One-shot: rename BOOTSTRAP.md → BOOTSTRAP.md.done
                if *one_shot {
                    let done_path = workspace_dir.join(format!("{filename}.done"));
                    if let Err(e) = std::fs::rename(&path, &done_path) {
                        warn!(
                            "workspace identity: failed to rename {filename} to .done: {e}"
                        );
                    } else {
                        debug!("workspace identity: renamed {filename} → {filename}.done");
                    }
                }
            }
            Err(e) => {
                warn!("workspace identity: failed to store {key}: {e}");
            }
        }
    }

    if written > 0 {
        info!("workspace identity: hydrated {written} identity entries into memory");
    }

    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::traits::MemoryEntry;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MockMemory {
        store: Mutex<HashMap<String, String>>,
    }

    impl MockMemory {
        fn new() -> Self {
            Self {
                store: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl Memory for MockMemory {
        fn name(&self) -> &str {
            "mock"
        }

        async fn store(
            &self,
            key: &str,
            content: &str,
            _category: MemoryCategory,
            _session_id: Option<&str>,
        ) -> anyhow::Result<()> {
            self.store
                .lock()
                .unwrap()
                .insert(key.to_string(), content.to_string());
            Ok(())
        }

        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(vec![])
        }

        async fn get(&self, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
            Ok(self.store.lock().unwrap().get(key).map(|content| {
                MemoryEntry {
                    key: key.to_string(),
                    content: content.clone(),
                    category: MemoryCategory::Core,
                    score: None,
                }
            }))
        }

        async fn list(
            &self,
            _category: Option<&MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(vec![])
        }

        async fn forget(&self, key: &str) -> anyhow::Result<bool> {
            Ok(self.store.lock().unwrap().remove(key).is_some())
        }

        async fn count(&self) -> anyhow::Result<usize> {
            Ok(self.store.lock().unwrap().len())
        }

        async fn health_check(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn hydrates_present_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("IDENTITY.md"), "You are TestBot.").unwrap();
        std::fs::write(dir.path().join("SOUL.md"), "Be helpful.").unwrap();

        let mem = MockMemory::new();
        let written = hydrate_workspace_identity(&mem, dir.path()).await.unwrap();

        assert_eq!(written, 2);
        assert!(mem.get("agent:identity").await.unwrap().is_some());
        assert!(mem.get("agent:soul").await.unwrap().is_some());
        assert!(mem.get("agent:topology").await.unwrap().is_none()); // AGENTS.md not present
    }

    #[tokio::test]
    async fn idempotent_skips_existing_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("IDENTITY.md"), "You are TestBot.").unwrap();

        let mem = MockMemory::new();
        let first = hydrate_workspace_identity(&mem, dir.path()).await.unwrap();
        let second = hydrate_workspace_identity(&mem, dir.path()).await.unwrap();

        assert_eq!(first, 1);
        assert_eq!(second, 0); // already in memory
    }

    #[tokio::test]
    async fn bootstrap_renamed_after_hydration() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("BOOTSTRAP.md"),
            "First run instructions.",
        )
        .unwrap();

        let mem = MockMemory::new();
        hydrate_workspace_identity(&mem, dir.path()).await.unwrap();

        assert!(!dir.path().join("BOOTSTRAP.md").exists());
        assert!(dir.path().join("BOOTSTRAP.md.done").exists());
    }

    #[tokio::test]
    async fn bootstrap_done_skipped_on_subsequent_runs() {
        let dir = tempfile::tempdir().unwrap();
        // Simulate already-processed state
        std::fs::write(dir.path().join("BOOTSTRAP.md.done"), "done").unwrap();

        let mem = MockMemory::new();
        let written = hydrate_workspace_identity(&mem, dir.path()).await.unwrap();

        assert_eq!(written, 0);
    }
}
