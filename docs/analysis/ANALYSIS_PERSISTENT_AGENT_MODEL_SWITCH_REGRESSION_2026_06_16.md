# ANALYSIS: Model switch silently no-ops on running Claude agents (regression)

**Date:** 2026-06-16
**Author:** Naki
**Severity:** regression — `/model`, `/effort`, `/permission-mode` (and the inline pickers)
silently stopped taking effect on a running Claude agent.
**Regressing commit:** `a05060b0` (PR #1451, 2026-06-15) — *"feat(agent): answer AskUserQuestion
via the Agent SDK control protocol"*

---

## 1. Symptom

User changes the model (e.g. Opus → Sonnet) on a running Claude agent pane; the agent keeps
responding on the old model. "It was working before."

## 2. Root cause

The model/effort/permission are passed to the agent CLI as **spawn-time flags** (`--model`,
`--effort`, permission flags) built by `buildRuntimeArgs` from `block.meta["agent:runtime"]`.

- **Subprocess controllers** (`controllerType: "subprocess"` — codex, gemini, …) spawn a *fresh
  process per turn with `--resume`* (`providers/index.ts:75`), so a runtime change is picked up on
  the next turn. The slash/picker handlers say *"applies to next turn"* — true here.
- **Persistent controllers** (`controllerType: "persistent"` — Claude stream-json) spawn **one
  process on the first message and keep it alive** across turns
  (`backend/blockcontroller/persistent.rs:8-11`); turns just write to stdin. The frontend still
  rewrites `cmd:args` every turn (`hooks/useAgentCommands.ts:325-333`) and the runtime handler still
  writes `agent:runtime` (`commands/global/runtime.ts`), but **nothing restarts the live process**,
  so the new `--model` never takes effect.

`resync_controller(force=true)` (`blockcontroller/mod.rs:367-379`) *does* cleanly restart a
controller — but no code called it on a runtime change, and the persistent controller had **no
resume support** (no `resume_flag` in `PersistentSpawnConfig`; line 16's "auto-restart via
session_id" was aspirational), so even a restart would have dropped the conversation.

## 3. How it regressed

PR #1451 / `a05060b0` (2026-06-15) flipped Claude from `subprocess` → `persistent`
(`providers/index.ts:170`, `git blame`) to add the Agent SDK control protocol (AskUserQuestion /
mid-turn steering). That switch silently invalidated the *"applies on next turn"* assumption the
runtime-change path relies on — but the runtime path was never updated to restart persistent
controllers. So model/effort/permission changes have silently no-op'd on Claude agents since
2026-06-15.

## 4. Evidence (live trace of this Naki pane)

srv v0.46.0 log, block `91580ebe-5917-419b-b783-cd242ea2c9af`:
- `21:35:38` `persistent process spawned … args=[… "--model","opus","--effort","xhigh"]` — **once**.
- `23:04:47` `SetMeta agent:runtime`, `23:05:04`/`23:05:58` `SetMeta cmd:args` — the model change *is*
  written to meta.
- **No** `ControllerResync` / spawn / stop after `21:35:38`; the string `sonnet` never appears in the
  log; the only live model token is `claude-opus-4-8`. → meta updated, live process unchanged.

## 5. Fix

**Frontend** (`commands/global/runtime.ts`): after writing `agent:runtime`, if the controller is
persistent, rebuild `cmd:args` (same `buildRuntimeArgs` the per-turn path uses) and call
`ControllerResyncCommand{forcerestart:true}`. `resync_controller` swaps in a fresh controller
instance (no process-waiter race), and the change applies immediately.

**Server — resume parity** (`blockcontroller/persistent.rs`, `server/websocket.rs`,
`server/app_api.rs`): `PersistentSpawnConfig` gains `resume_flag` + `session_id` (read from
`agent:resume_flag` / `agent:sessionid` meta — the same keys the subprocess path already reads).
`spawn_process` hydrates `inner.session_id` and appends `--resume <sid>` when present — a verbatim
mirror of `SubprocessController::spawn_turn` (subprocess.rs:364-378). So the forced restart respawns
Claude with the new `--model` **and** resumes the same conversation (no context loss).

Net: subprocess behavior is unchanged; persistent agents now apply runtime changes via a
resume-preserving restart — restoring the pre-#1451 behavior.

## 6. Testing

- `cargo check -p agentmux-srv` green; `tsc --noEmit` clean for `runtime.ts`.
- ⚠️ Needs a `task dev`/`task package` (CEF) smoke test: change `/model` on a running Claude pane →
  confirm the next turn runs on the new model and the conversation continues (resume).

## 7. Follow-ups

- Consider restarting only when a *spawn-affecting* field actually changed (model/effort/permission)
  to avoid an unnecessary respawn when the picker re-confirms the current value.
- The slash/picker success messages still say "applies to next turn"; for persistent it now restarts
  — wording could be tightened.
