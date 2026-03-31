# Design Plan: ZeroClaw ↔ Mission Control Integration

**Status:** Phase 1 implemented (2026-03-31)
**Risk tier:** Medium (new observer backend + config keys; no security boundary changes)
**Depends on:** Mission Control API running at a known base URL

---

## Goal

Connect a ZeroClaw instance to Mission Control so it:

1. Appears **online** in the dashboard (heartbeat signal)
2. Reports **task lifecycle events** in real-time (hook signals)
3. Can **receive directives** from super-master (inbox poll)
4. Reads its **Program** at session start to align with current priorities

---

## Architecture Overview

```
ZeroClaw runtime
│
├── MissionControlObserver  ←─ implements Observer trait
│   ├── record_event(AgentStart)       → POST /api/v1/signals/events  (task.status / agent.lifecycle)
│   ├── record_event(AgentEnd)         → POST /api/v1/signals/events  (task.status done)
│   ├── record_event(HeartbeatTick)    → POST /api/v1/signals/heartbeat
│   ├── record_event(Error)            → POST /api/v1/signals/events  (error.runtime)
│   └── record_event(TurnComplete)     → POST /api/v1/signals/events  (task.status in_progress)
│
├── Cron scheduler
│   └── every 15 min → HeartbeatTick  (fallback if observer misses idle windows)
│
└── Agent loop (session start)
    └── GET /api/v1/programs/<INSTANCE_ID>  → prepend Program to system context
```

All outbound HTTP calls use a shared non-blocking `reqwest` client with a 5-second timeout and fire-and-forget (errors are logged, never bubble up to the agent).

---

## New Files

| File | Purpose |
|------|---------|
| `src/observability/mission_control.rs` | `MissionControlObserver` — the new Observer backend |
| `src/observability/mission_control_client.rs` | Thin async HTTP client wrapping Mission Control API calls |

---

## Changed Files

| File | Change |
|------|--------|
| `src/observability/mod.rs` | Register `"mission-control"` backend in `create_observer` factory |
| `src/observability/traits.rs` | No change needed — existing events map cleanly |
| `src/config/mod.rs` (or schema file) | Add `[mission_control]` config section |
| `src/agent/` (session start) | Read Program from MC API and prepend to context |

---

## Config Schema

Add to `zeroclaw.toml`:

```toml
[mission_control]
enabled = false                             # opt-in
api_base = "http://127.0.0.1:4010"
auth_token = ""                             # set via env: MISSION_CONTROL_AUTH_TOKEN
instance_id = ""                            # set via env: INSTANCE_ID
agent_id = "manager"                        # which agentId to report as
emit_task_signals = true                    # POST task.status events
emit_heartbeat = true                       # POST instance.heartbeat on HeartbeatTick
read_program_on_start = true                # GET /api/v1/programs/<instance_id> at session start
poll_inbox = false                          # GET /api/v1/agents/<id>/inbox (opt-in)
poll_inbox_interval_secs = 60
```

Environment variable overrides (matching Mission Control's onboarding spec):
- `MISSION_CONTROL_API_BASE` → `api_base`
- `MISSION_CONTROL_AUTH_TOKEN` → `auth_token`
- `INSTANCE_ID` → `instance_id`

---

## Signal Mapping

### ObserverEvent → Mission Control signal kind

| `ObserverEvent` variant | MC `kind` | MC `state` | Notes |
|-------------------------|-----------|-----------|-------|
| `AgentStart` | `agent.lifecycle` | `in_progress` | session started |
| `AgentEnd` (success) | `task.status` | `done` | |
| `AgentEnd` (error path) | `task.status` | `failed` | |
| `TurnComplete` | `task.status` | `in_progress` | progress update |
| `HeartbeatTick` | `instance.heartbeat` | `idle` | routed to `/heartbeat` endpoint |
| `Error` | `error.runtime` | — | only if `component != "provider"` to avoid noise |
| `ChannelMessage` inbound | skipped | — | too noisy, not meaningful to MC |

Task-level fields (`taskKey`, `projectCode`, `changeId`, `phase`) are **not** inferred from observer events — they require agent-driven context. Phase 2 (see below) can add explicit task API calls.

---

## Implementation Phases

### Phase 1 — Heartbeat + Presence (no code change to agent loop)

Scope: observer backend only. Zero impact on agent behavior.

**Steps:**
1. Add `[mission_control]` config section to the config schema struct.
2. Implement `MissionControlClient` — a thin struct holding `api_base`, `auth_token`, `instance_id`, `agent_id` and a `reqwest::Client`. Methods:
   - `async fn post_event(&self, payload: Value) -> Result<()>`
   - `async fn post_heartbeat(&self, payload: Value) -> Result<()>`
3. Implement `MissionControlObserver`:
   - Holds `Arc<MissionControlClient>` + Tokio runtime handle for fire-and-forget spawns.
   - `record_event(HeartbeatTick)` → spawn `client.post_heartbeat(...)`.
   - `record_event(AgentStart | AgentEnd | Error)` → spawn `client.post_event(...)`.
   - All other variants → no-op for now.
4. Register `"mission-control"` in `create_observer` factory in `mod.rs`.
5. Write unit tests for signal mapping logic (no network — mock the client).
6. Write integration test: spin up a mock HTTP server, verify correct payloads.

**Validation:** `cargo test -p zeroclaw observability::mission_control`

---

### Phase 2 — Task Lifecycle (agent-driven)

Scope: agent loop + new task context carrier. Medium complexity.

**Steps:**
1. Define a `TaskContext` struct (optional fields: `task_key`, `project_code`, `change_id`, `phase`).
2. Thread `Option<TaskContext>` through the agent session (session-local state, not persisted).
3. When the agent emits a tool call or turn that sets task context (via a special tool or parsed from message), update `TaskContext` in session state.
4. Pass `TaskContext` into `record_event` calls so the observer can include `taskKey` etc. in payloads.
   - This requires adding an optional context parameter to `record_event` or a session-scoped observer wrapper.
5. Add `POST /api/v1/tasks` call on session start (if `task_key` is known).
6. Add `PATCH /api/v1/tasks/<key>` calls on phase transitions.

**Note:** This phase requires more design around how the agent communicates task identity. Defer until Phase 1 is validated in production.

---

### Phase 3 — Program Injection (session start)

Scope: agent loop only. Low risk.

**Steps:**
1. At session start, if `read_program_on_start = true`, call `GET /api/v1/programs/<instance_id>`.
2. If response is non-empty markdown, prepend it to the system prompt as:
   ```
   ## Mission Control Program
   <program content>
   ```
3. If the request fails (network error, 404), log a warning and continue — never block session start.
4. Cache the program for the session duration (re-fetch only on new session).

---

### Phase 4 — Inbox Polling (command reception)

Scope: background task. Low risk, opt-in.

**Steps:**
1. If `poll_inbox = true`, spawn a background task that GETs `/api/v1/agents/<instance_id>/<agent_id>/inbox` every `poll_inbox_interval_secs`.
2. For each returned command, dispatch it as a synthetic inbound message to the agent loop.
3. The endpoint deletes commands atomically on retrieval — no deduplication needed.

---

## Security Considerations

- `auth_token` must never be logged. Mask it in `GET /api/config` output (already done for other secrets via the `***MASKED***` pattern in the config API).
- `allow_public_bind` is unchanged — Mission Control calls out, not in. No gateway exposure needed for this integration.
- Fire-and-forget HTTP calls must not carry prompt content or tool arguments — only lifecycle metadata. The `ObserverEvent` design already enforces this (no prompt fields on events).
- If `api_base` resolves to a non-local address, log a one-time warning at startup.

---

## Testing Plan

| Test | Type | What it checks |
|------|------|----------------|
| `mission_control_client_post_event` | unit (mock server) | correct URL, headers, JSON shape |
| `mission_control_observer_heartbeat` | unit | `HeartbeatTick` → heartbeat endpoint |
| `mission_control_observer_agent_lifecycle` | unit | `AgentStart`/`AgentEnd` → events endpoint |
| `mission_control_observer_error_filter` | unit | provider errors are suppressed |
| `mission_control_observer_disabled` | unit | no requests when `enabled = false` |
| `config_mission_control_env_override` | unit | env vars override toml values |
| `phase1_e2e_heartbeat` | integration | full daemon loop emits heartbeat to real MC instance |

---

## Rollout Checklist

```
[ ] Phase 1 implemented and tests passing
[ ] `enabled = false` by default (opt-in)
[ ] auth_token masked in config API output
[ ] Declare instance in instances.yaml (MC side)
[ ] Set MISSION_CONTROL_AUTH_TOKEN + INSTANCE_ID env vars
[ ] Set observability.backend = "mission-control" in zeroclaw.toml
[ ] Restart ZeroClaw daemon
[ ] Confirm status shows "online" in MC dashboard within 60s
[ ] Phase 2 design reviewed before starting
```

---

## Related

- `src/observability/traits.rs` — Observer trait and event enum
- `src/observability/mod.rs` — observer factory
- `src/observability/otel.rs` — reference implementation for async HTTP observer
- `/docs/reference/api/config-reference.md` — config schema conventions
- Mission Control: `onboarding-new-instance.md` — signal API spec
- Mission Control: `api-contract.md` — full API reference
