# SPEC: MCP `Loop` / `LoopStop` tools — recurring prompt injection

**Date:** 2026-06-16
**Status:** Implemented
**Author:** Naki
**Related:** Claude Code's `/loop` skill (the analogue this mirrors); MuxBus `SendMessage`
tool + `/agentmux/reactive/inject` (the delivery path reused)

---

## 1. Goal

Give AgentMux agents the equivalent of Claude's `/loop` command: run a prompt or slash command on a
recurring interval. Surfaced as two new `agentmux-mcp` tools so any agent (including me, once built)
gets them alongside `Shell`/`SendMessage`/`OpenEditor`.

## 2. Design

A loop is just *"re-inject this prompt into a conversation every N"*. The delivery primitive already
exists — `POST /agentmux/reactive/inject` (`reactive_handler.inject_message`), the same path
`SendMessage` uses, which routes a message to a target agent's active conversation (local → LAN →
cloud tiers) and supports `wait_for_idle`.

**Where the loop lives: the agentmux-mcp process.** Unlike `Shell` (whose output must stream into a
conversation document, so its lifecycle lives in srv), a loop has no state beyond "fire the inject on
a timer." Claude's `/loop` is likewise driven by the tool/client layer, not the backend. So each loop
is an in-process `tokio` task in the MCP server:

```
loop { if !immediate { sleep(interval) first };  POST inject(prompt → target, wait_for_idle); sleep(interval) }
```

- A `LoopRegistry = Mutex<HashMap<loop_id, JoinHandle>>` (created in `main`, passed to `call_tool`)
  holds running loops. `loop_id` = `loop-<n>` from an `AtomicU64`.
- The MCP process is long-lived for the agent session, so **loops stop automatically when the agent
  pane / session ends** (the process dies, tasks with it) — the correct lifecycle, no orphans.
- `LoopStop(loop_id)` removes the handle and `.abort()`s the task.

**Why not srv:** would mean new endpoints + an AppState registry + scheduler tasks for a feature that
is purely a timer over an existing endpoint. Keeping it in the MCP process is smaller, has no srv
surface, and matches the client-driven nature of `/loop`. (Trade-off: loops don't persist across an
MCP-process restart — acceptable; a loop is inherently session-scoped. srv-side persistence is a
possible follow-up if cross-restart loops are wanted.)

## 3. Tools

**`Loop`** — args: `prompt` (required), `interval` (string `30s`/`5m`/`1h`; bare number = minutes;
default `10m`; clamped [10s, 24h]), `to` (target agent name; default self via `AGENTMUX_AGENT_ID`),
`immediate` (run once on start too; default false). Returns `loop-<n>`. Injects with
`wait_for_idle: true` so a re-prompt lands when the target finishes its turn rather than interrupting.

**`LoopStop`** — args: `loop_id`. Aborts the task; idempotent ("not running" if unknown).

## 4. Files

| File | Change |
|---|---|
| `agentmux-mcp/Cargo.toml` | add tokio `time` feature (for `sleep`) |
| `agentmux-mcp/src/main.rs` | `LOOP_TOOL`/`LOOP_STOP_TOOL` schemas; `LoopRegistry`; registry in `main`; `Loop`/`LoopStop` arms in `call_tool`; `parse_interval` |

No srv changes. `cargo check -p agentmux-mcp` green.

## 5. Testing

- `cargo check -p agentmux-mcp` — green.
- Manual (needs an AgentMux build): `Loop(prompt:"say hi", interval:"30s")` → the agent is re-prompted
  every 30s; `LoopStop(loop-1)` stops it; closing the pane stops all loops.

## 6. Notes / follow-ups

- `parse_interval` accepts `s`/`m`/`h` + bare-number-as-minutes; min 10s guards against runaway loops.
- Self-target relies on `AGENTMUX_AGENT_ID` being set (it is, on the agent process env). A `to`
  targets another agent — a recurring nudge to a peer.
- Possible follow-ups: a `LoopList` tool, `max_iterations`, and srv-side persistence across restarts.
