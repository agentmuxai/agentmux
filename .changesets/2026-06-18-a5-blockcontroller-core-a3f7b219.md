---
type: patch
---

refactor(A5): extract BlockControllerCore shared helpers; fix ACP session-id persist

Extracts duplicated logic from the four block controllers into a new
`blockcontroller/core.rs` module:

- `apply_working_dir()` — tilde-expand, mkdir, current_dir, env-var setup
  (replaces ~35-line copy-paste in persistent/subprocess/acp)
- `spawn_health_watchdog()` — 5-second poll loop
  (was duplicated twice in subprocess.rs)
- `persist_session_id()` — persist `agent:sessionid` to block metadata
  and broadcast `waveobj:update` so the frontend reflects the change
- `META_SESSION_ID` — `"agent:sessionid"` constant (was magic string)

Bug fix: AcpController was capturing the session ID in memory
(`inner.session_id`) but never persisting it to block metadata or
broadcasting the update. This broke the "My Agents" reattach path
for ACP agents (frontend reads `block.meta["agent:sessionid"]` to
resume with `--resume <sid>`). The fix routes ACP through
`core::persist_session_id`, matching the careful path from
persistent.rs and subprocess.rs.

Net: –249 lines removed from the four controller files.
