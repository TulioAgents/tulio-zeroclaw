# ZeroClaw Fixes — 2026-04-01

## Summary

Full debugging session to get the ZeroClaw agent chat UI (web dashboard) working with real tool execution, file creation, and agent delegation.

---

## Issues Fixed

### 1. Wrong provider — `anthropic-custom` → `minimax`
- **Problem:** Config used `anthropic-custom:https://api.minimax.io/anthropic`, which requires `ANTHROPIC_API_KEY`. The MiniMax key is `MINIMAX_API_KEY`.
- **Fix:** Replaced all `anthropic-custom:https://api.minimax.io/anthropic` occurrences in `~/.zeroclaw/config.toml` with `minimax`. Also removed the bogus `minimax/` prefix from the `basic` model route.

### 2. `MINIMAX_API_KEY` missing from daemon environment
- **Problem:** Key was exported in `~/.zshrc` but launchd doesn't source shell profiles.
- **Fix:** Added `MINIMAX_API_KEY` and `MISSION_CONTROL_AUTH_TOKEN` directly to `~/Library/LaunchAgents/com.zeroclaw.daemon.plist` under `EnvironmentVariables`.

### 3. Chat UI used a no-tools code path (root cause of hallucination)
- **Problem:** `/ws/chat` (used by the web dashboard) called `provider.chat_with_history()` directly — a plain LLM call with no tool loop. Telegram/Discord used `crate::agent::process_message()` which runs the full agentic loop with tool execution.
- **Fix:** Changed `src/gateway/ws.rs` to call `crate::agent::process_message()` instead of `chat_with_history()`. Rebuilt with `cargo build --release`.
- **File:** `src/gateway/ws.rs` ~line 202

### 4. Homebrew binary was stale
- **Problem:** `/opt/homebrew/bin/zeroclaw` symlinked to old Cellar binary that crashed on startup.
- **Fix:** Repointed symlink: `ln -sf ~/agents/tulio-zeroclaw/target/release/zeroclaw /opt/homebrew/bin/zeroclaw`. Updated launchd plist `ProgramArguments` to use `/Users/javierhbr/agents/tulio-zeroclaw/target/release/zeroclaw` directly.

### 5. Agent hallucinating tool calls as text
- **Problem:** MiniMax model outputs its own `<minimax:tool_call><invoke>` format. ZeroClaw's XML dispatcher only parses `<tool_call>{"name":...}</tool_call>`. The model had no explicit instruction about which format to use.
- **Fix:** Injected an `!! ABSOLUTE RULE — TOOL CALL FORMAT !!` block at the top of all 8 agents' `IDENTITY.md` files showing the exact format with concrete examples. Must be at the very top (before SOUL.md is loaded) to take effect.
- **Agents patched:** `cto`, `po`, `tech-lead`, `staff-fullstack`, `sr-fullstack`, `mobile`, `qa`, `devops`

### 6. `file_write` and `shell` blocked in supervised mode
- **Problem:** `auto_approve` in `~/.zeroclaw/config.toml` only contained `["file_read", "memory_recall", "delegate"]`. In `supervised` mode, `file_write` and `shell` required interactive approval — impossible over a WebSocket session. Agent would silently stop.
- **Fix:** Updated `auto_approve` to `["file_read", "file_write", "shell", "memory_recall", "memory_store", "delegate", "git"]`.

### 7. Agents missing `file_write` in `allowed_tools`
- **Problem:** `cto`, `tech-lead`, `devops` were missing `file_write`. `po` was missing `shell` and `git`.
- **Fix:** All 8 agents now have `["memory_recall", "memory_store", "file_read", "file_write", "shell", "git", "delegate"]`.

### 8. `allowed_commands` missing `mkdir`, `touch`, `cp`, `mv`
- **Problem:** Shell autonomy only allowed `git`, `npm`, `cargo`, `ls`, `cat`, `grep`, etc. No `mkdir` — agent couldn't create directories.
- **Fix:** Added `mkdir`, `touch`, `cp`, `mv` to `allowed_commands` in `[autonomy]`.

### 9. `stop-daemon.sh` didn't stop launchd-managed service
- **Problem:** Script used `pkill -f "zeroclaw daemon"` which killed the process but launchd immediately restarted it.
- **Fix:** Replaced with `launchctl unload ~/Library/LaunchAgents/com.zeroclaw.daemon.plist` + port 42617 cleanup as fallback.

### 10. Mission Control `agent_id` mismatch
- **Problem:** `config.toml` had `agent_id = "Guaripolo"` but `instances.yaml` for `org-development` uses `manager`.
- **Fix:** Changed to `agent_id = "manager"` in `~/.zeroclaw/config.toml`.

---

## Key Files Changed

| File | What changed |
|------|-------------|
| `~/.zeroclaw/config.toml` | Provider → `minimax`, `auto_approve` expanded, `allowed_commands` expanded, `agent_id` fixed, all agent `allowed_tools` updated |
| `~/Library/LaunchAgents/com.zeroclaw.daemon.plist` | Added `MINIMAX_API_KEY`, `MISSION_CONTROL_AUTH_TOKEN`; binary path → local build |
| `src/gateway/ws.rs` | WS chat now calls `process_message()` instead of `chat_with_history()` |
| `src/gateway/mod.rs` | Webhook also updated to `run_gateway_chat_with_tools()` |
| `~/.zeroclaw/workspace/agents/*/IDENTITY.md` | Tool call format rule injected at top of all 8 agents |
| `~/.zeroclaw/workspace/agents/*/SOUL.md` | Real tool execution rule added |
| `/opt/homebrew/bin/zeroclaw` | Symlink repointed to local build |
| `stop-daemon.sh` | Uses `launchctl unload` instead of `pkill` |

---

## Result

After all fixes, the web dashboard chat sends messages through the full agentic loop. HelloWorld6 and HelloWorld7 successfully ran — created OpenSpec structure, wrote files, and delegated across agents (cto → sr-fullstack → qa).
