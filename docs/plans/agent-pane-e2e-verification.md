# Agent Pane E2E Verification — Status

**Updated:** 2026-04-12

This plan is **mostly complete**. Full end-to-end agent pane interaction has been verified through v0.33.91.

## Verified

- ✅ `agent.open` creates block, inserts layout node, registers controller
- ✅ Pane visible in UI immediately
- ✅ `agent.send` delivers messages to the persistent Claude Code process
- ✅ Multi-turn conversation with preserved session ID (no respawn)
- ✅ Responses render in the pane via `useAgentStream` → translator → parser
- ✅ `/login` intercepted frontend-side → `runCliLogin` → OAuth URL displayed with hover-to-copy
- ✅ `/clear` frontend-only document reset
- ✅ Agent pane launched via `open_agent` IPC (frontend-driven path)

## Remaining

- Typing smoothness — primary fix shipped in 0.33.91 (uncontrolled textarea). Secondary optimizations tracked in `docs/analysis/agent-pane-typing-lag-2026-04-12.md`.
- Tool execution end-to-end (Bash/Edit/Grep from inside agent pane) — should work via the persistent stdin path but not explicitly tested.
- `agent.stop` cancel-mid-stream — not tested yet.

## Related Docs

- `docs/specs/app-api-extension.md` — full App API spec
- `docs/specs/app-api-status.md` — implementation status per command
- `docs/analysis/agent-pane-typing-lag-2026-04-12.md` — typing lag root cause + fixes
- `docs/analysis/persistent-process-retro-2026-04-10.md` — debug history for persistent mode
