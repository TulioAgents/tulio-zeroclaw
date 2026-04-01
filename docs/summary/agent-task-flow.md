# Agent Task Flow — Build Log

> Session date: 2026-04-01
> Branch: tulio-flavor
> Goal: Autonomous multi-agent software delivery pipeline driven by ZeroClaw.

---

## What We Wanted

A fully autonomous pipeline where a single queue item triggers a chain of agents:

```
guaripolo (CTO) → po → tech-lead → sr-fullstack → qa → devops
```

Each agent completes its step, writes OpenSpec artifacts, updates `tasks.md`, and hands off to the next agent — no manual intervention between steps. The Kanban board should reflect live progress driven entirely by agent activity.

---

## What We Built

### 1. `queue_enqueue` Tool (`src/tools/queue_enqueue.rs`)

A new tool allowing agents to enqueue tasks for other agents via the persistent SQLite queue.

**Signature:**
```
queue_enqueue(agent, project, task_id, task, context?, priority?)
```

The `project` and `task_id` fields are embedded into the `context` column as structured lines:
```
project: helloworld3
task_id: hw3-004
```

This enables the `zeroclaw task status` command to cross-reference queue items against `tasks.md` rows.

**Worked:** Tool registered, security policy wired, auto-approved in config.

---

### 2. Queue Drain — Agent Identity (`src/queue/mod.rs`)

**Problem:** Queue drain always passed the global config to `agent::run()`, so every agent ran with the root workspace identity — no `IDENTITY.md`, no `AGENTS.md`, no role awareness.

**Fix:** Derive `agent_workspace = workspace_dir/agents/<agent>` and override `agent_config.workspace_dir` before calling `run()`. Each agent now loads its own identity files.

**Worked:** Agents correctly pick up their `IDENTITY.md`, `SOUL.md`, `AGENTS.md` on drain.

---

### 3. Guaripolo = CTO + Dev Manager (merged roles)

**Problem:** `DelegateTool` hard-removes itself from sub-agent toolsets (line 415 of `delegate.rs`), so delegation only goes 1 level deep. A manager agent between guaripolo and the engineers couldn't forward work further.

**Fix:** Merged CTO and Dev Manager into a single `guaripolo` role. Guaripolo owns the full pipeline: receives project intent, delegates directly to each agent in sequence via `queue_enqueue`.

**Worked:** One agent orchestrates the whole chain without the 1-level delegation limit.

---

### 4. MiniMax via Anthropic-Compatible Endpoint

**Problem:** The native MiniMax provider forced `merge_system_into_user=true` → `native_tool_calling=false`. Agents couldn't use tools.

**Fix:** Use `anthropic-custom:https://api.minimax.io/anthropic` instead. This endpoint is Anthropic-protocol-compatible, gets `native_tool_calling=true`, and resolves credentials via the `ZEROCLAW_API_KEY` generic fallback.

**Worked:** All agents use tools (file_read, file_write, queue_enqueue, delegate) correctly.

---

### 5. File Write Permissions (`workspace_only`, `allowed_roots`)

**Problem:** Agents wrote files to `~/.zeroclaw/workspace/helloworld2/` instead of `~/coding-projects/helloworld2/` because `workspace_only = true` blocked writes outside the workspace directory.

**Fix:**
```toml
workspace_only = false
allowed_roots = ["/Users/javierhbr/coding-projects"]
```

**Worked:** Agents write OpenSpec artifacts and source files to the correct project directories.

---

### 6. Daemon Management Scripts

Added `start-daemon.sh` and `stop-daemon.sh` to prevent duplicate daemon processes (port 42617 conflicts).

`start-daemon.sh` kills any existing process before starting, sources `.zshrc` to pick up `ZEROCLAW_API_KEY`, and runs the debug build.

**Worked:** Clean single-instance daemon lifecycle.

---

### 7. Dynamic Kanban — Auto-Discovery

**Problem:** Kanban returned a hard error when `project-map.yaml` was missing or empty.

**Fix:** Made `project-map.yaml` optional. Added auto-discovery scan of `~/coding-projects/*/openspec/changes/` — any project directory with an `openspec/changes/` folder is surfaced automatically.

**Worked:** Kanban shows agent-created projects without manual registration.

---

### 8. Kanban — Close/Hide Projects

Added `status: closed` support. Projects registered in `project-map.yaml` with `status: closed`:
- Skip task loading in the backend (`api.rs`)
- Are filtered from the project tab buttons in the frontend (`KanbanBoard.tsx`)

Usage: edit `~/coding-projects/project-map.yaml`:
```yaml
projects:
  - name: helloworld2
    path: ~/coding-projects/helloworld2
    status: closed
```

**Worked:** Closed projects disappear from Kanban without deleting any files.

---

### 9. `zeroclaw task status` CLI Command

New command to compare `tasks.md` (file truth) against the queue DB (execution truth) and surface drift.

```
zeroclaw task status --project helloworld3 --change hw3-001
zeroclaw task status --project helloworld3 --change hw3-001 --sync
```

**Output columns:** Task ID, Title, Owner, File status, Queue status, Drift marker.

**`--sync` flag:** Patches `tasks.md` rows where the queue shows `done/in-progress/failed` but the file still says `todo`. Queue is treated as the authoritative status.

**New API in `queue/mod.rs`:** `query_by_project(workspace_dir, project, task_id?)` — single DB call returning all queue items for a project, optionally filtered by `task_id` line in context.

**Worked:** Command compiles, runs, correctly detects drift.

---

### 10. Task Completion Protocol (Agent Instructions)

All agents (`po`, `tech-lead`, `sr-fullstack`, `qa`, `devops`, `cto`) had the following appended to their `AGENTS.md`:

**Task completion protocol (REQUIRED):**
1. Update `tasks.md` — flip your row from `todo` → `done`
2. Write `handoff.md` with: project, change ID, owner, what done, what blocked, next step
3. Call `queue_enqueue` for the next agent

**Why needed:** Without explicit instructions, agents completed their work but never updated `tasks.md`. The Kanban showed tasks stuck in "To Do" even after the queue item was `done`.

---

### 11. TASK GUARD in `build_drain_prompt()`

When a queue item's context contains `project:` and `task_id:` lines, the drain prompt now injects a `[TASK GUARD]` block telling the agent:
- Which `tasks.md` row it owns
- To update `in-progress` on start and `done` on finish
- To use the **next agent's task row ID** as `task_id` in `queue_enqueue` — not the current queue item UUID

**Why needed:** Agents were reporting completion back to guaripolo with `task_id: pipeline-test-001` (the original trigger item ID) instead of their own task row ID (e.g. `hw3-003`). This broke the queue↔tasks.md cross-reference.

---

## What Didn't Work

### Delegation chain deeper than 1 level

`DelegateTool` removes itself from sub-agent toolsets. A `manager → engineer` chain fails because the manager's sub-agents have no `delegate` tool to continue forwarding. The workaround (merge manager into guaripolo) works, but true multi-level delegation is not supported without code changes to `delegate.rs`.

### Queue items not mapped per-task (legacy runs)

Pipelines triggered before the TASK GUARD fix used `task_id: pipeline-test-001` for all callback items. The `zeroclaw task status` command shows "drift" for all rows in those projects because no queue item matches their individual task IDs. This is expected for old runs — new pipeline executions will have correct per-task IDs.

### Agents updating wrong tasks.md rows

Early runs had agents writing status updates to rows by position or hardcoded strings instead of by task ID. Fixed by the TASK GUARD injection in `build_drain_prompt()` and the explicit protocol in `AGENTS.md`.

### Rate limiting blocking pipeline completion

`max_actions_per_hour = 20` was too low for a 9-agent pipeline executing in sequence. Each agent run consumes multiple actions (file reads, tool calls, queue enqueue). Raised to `200`.

### `.zshrc` parse error blocking daemon start

A corrupt byte sequence after the `OPENROUTER_API_KEY` line caused `unmatched "` syntax errors, which prevented `source ~/.zshrc` from setting `ZEROCLAW_API_KEY`. Fixed by rewriting the corrupted line cleanly with `od -c` inspection.

---

## End State

A full autonomous software delivery pipeline verified with the HelloWorld3 project:

```
queue item → guaripolo → po (hw3-001) → tech-lead (hw3-002)
          → sr-fullstack (hw3-003) → qa (hw3-004)
```

All 5 queue items reach `done` status. OpenSpec artifacts (`proposal.md`, `tasks.md`, `handoff.md`) written by agents. Source file (`helloworld3.html`) implemented and QA-verified. Kanban board reflects task status from `tasks.md` in real time.

The `zeroclaw task status` command provides a CLI audit tool to detect and fix drift between file state and queue execution state going forward.
