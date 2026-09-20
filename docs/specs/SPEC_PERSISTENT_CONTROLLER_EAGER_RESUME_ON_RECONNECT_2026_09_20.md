# SPEC — Eagerly resume a persistent agent controller that already has a session, instead of waiting for the next message

**Date:** 2026-09-20
**Author:** Agent3
**Status:** proposed
**Triggered by:** `docs/incident/INCIDENT_2026_09_20_APP_CLOSED.md` — during that
investigation, 3 of 7 agent panes in one window (Clamk, Naki #2, Loap #2)
turned out to have no live CLI process at all after the app's repeated
restarts, silently, with no error and no visible difference in the UI's
`view: "agent"` pane — they were sitting inert, waiting for a message nobody
had sent since the restart. Diagnosed live in that conversation; this spec is
the fix.

---

## 1. The bug, precisely

`agentmux-srv/src/backend/blockcontroller/mod.rs`'s `resync_controller()` —
called for every pane on `ControllerResync` (fired on window/tab load,
reconnect, and backend restart) — has two paths that matter here:

1. **An existing in-memory controller found `STATUS_DONE`:** honors the
   caller's `respawn_if_done` flag (`server/websocket.rs:1025`, defaulted
   `true` for the general case via `!cmd.norespawn`) and calls `ctrl.start()`
   if set.
2. **No existing in-memory controller at all** (`mod.rs:836` onward,
   `"Create new controller"`) — this is the path that actually runs after a
   full backend restart, since a fresh `srv` process has no in-memory
   controller state for anything. **`respawn_if_done` is never consulted
   here at all** — a controller is unconditionally constructed and
   `ctrl.start()` is called on it, for every controller type.

Whether that `start()` actually resumes anything live depends entirely on
the controller type's own implementation, and here they diverge:

- **`ShellController::start()`** (`blockcontroller/shell/lifecycle.rs:297-320`)
  eagerly spawns the shell process right there (gated only by the block's
  own `run_on_start` meta flag, not by "has anyone sent a message yet"). This
  is exactly what the `norespawn` field's own doc comment
  (`rpc_types/block.rs:82-95`) describes as intentional: *"transparently
  reviving a shell that died in a crash or backend restart."*
- **`PersistentSubprocessController::start()`** (`blockcontroller/persistent.rs:4691-4701`)
  does none of that. It logs `"persistent controller registered (spawns on
  first message)"` and returns — no process, no `--resume`, nothing. This is
  the **entire** implementation of `start()` for this controller type. It
  behaves identically whether the block is a genuinely brand-new pane that
  has never run anything, or an existing pane with a full prior conversation
  and a captured `session_id` sitting in its meta, freshly orphaned by a
  crash/restart.

**Consequence:** every agent pane in the window survives a backend restart
as an inert shell — correct layout, correct chrome, `view: "agent"`, but no
process behind it — until a human or another agent happens to send it a
message. Nothing in the UI distinguishes this from a genuinely idle-but-alive
agent; `ListAgents` (a Claude-Code-specific cross-session discovery
mechanism, unrelated to AgentMux's own backend) simply can't see it either
way, which is what surfaced this during the incident investigation.

---

## 2. Why the lazy-spawn design exists at all — and why it doesn't apply here

`docs/specs/persistent-process-mode.md` (the original spec for this
controller type) specifies the state machine as `INIT ─(first message)─>
SPAWNING` from the start (line 73) — lazy spawn is deliberate, and it is
the RIGHT behavior for the case it was designed for: a freshly created pane
that has never been given a prompt yet has no reason to burn a process
before it's needed.

That same spec's line 236 explicitly leaves crash-recovery as an open
question: *"set status to CRASHED and either auto-respawn or show a
'reconnect'"* — **not a decision that auto-respawn was rejected**, just one
that was never made either way. No spec, retro, or comment found anywhere
in this codebase records a deliberate choice against eagerly resuming an
existing session (searched `docs/specs/`, `docs/retro/` for
"eager"/"auto-respawn"/"resume" near "persistent"/"agent"/"cost"/"token" —
nothing). Per this repo's own house rule about treating an undocumented
claim of a *rejected* idea with skepticism (see `CLAUDE.md`'s Jekt section
for the general form of this principle): there is no prior decision this
spec would be overturning. It is filling a gap the original spec explicitly
left open, not reversing one.

**The two cases are genuinely different and the fix should keep treating
them differently:**
- A pane with **no prior `session_id`** — nothing has ever been sent to it.
  Lazy spawn is still correct: don't burn a process on something that may
  never be used.
- A pane with an **existing `session_id`** in its meta (`agent:sessionid`,
  per `persistent.rs:286-289`'s own doc comment) — this pane has a real,
  ongoing conversation that was interrupted, not started. This is exactly
  the shell-controller's "transparently reviving" case, just for a
  different controller type.

---

## 3. The fix

`PersistentSubprocessController::start()` should check the block's
`agent:sessionid` meta (already a first-class, well-established mechanism
in this exact file — used today to hydrate `inner.session_id` before a
forced-restart or picker-reattach respawn, per the `session_id` field's own
doc comment at `persistent.rs:285-289`, `"Session id to hydrate
inner.session_id with BEFORE spawning, when the controller hasn't captured
one yet (fresh controller after a forced restart, or picker reattach)"`)
and branch:

- **`agent:sessionid` present and non-empty:** eagerly spawn now, exactly
  the same way a forced-restart/model-change respawn already does today —
  hydrate `inner.session_id` from it and spawn with `--resume <sid>`
  (`resume_flag`, per the existing `PersistentSpawnConfig` plumbing). This
  is not new spawn logic; it's triggering the SAME existing resume-spawn
  path this file already has, from one more call site.
- **`agent:sessionid` absent/empty:** keep today's behavior exactly —
  register and wait for the first message. No change for a genuinely new
  pane.

This requires `resync_controller`'s "Create new controller" branch
(`mod.rs:836` onward) to thread through whatever signal `start()` needs to
tell these two cases apart — in practice `start()` can likely read
`agent:sessionid` directly off the `block_meta` it's already passed, with
no new plumbing required at all; confirm this during implementation rather
than assume.

---

## 4. Open question to verify before shipping (not resolved by this spec)

**Does spawning `claude --resume <sid>` with no new prompt on the argv/stdin
make an API call on its own, or does it load session history and then sit
idle waiting for stdin, at zero cost until something is written to it?**
This spec assumes the latter (matching `persistent-process-mode.md`'s
described architecture: stdin is written per-turn, the process is kept
alive between turns doing nothing) but this was not independently verified
while writing this spec. If eager `--resume` spawn on its own *does*
trigger real work/cost, the fix needs a cheaper signal than a full process
spawn (e.g. read the session file directly to check "does history exist"
without invoking the CLI) — implementer should confirm this first.

---

## 5. Scope

**In scope:** `PersistentSubprocessController::start()`
(`agentmux-srv/src/backend/blockcontroller/persistent.rs`), and whatever
minimal change to `resync_controller`'s new-controller branch
(`agentmux-srv/src/backend/blockcontroller/mod.rs`) is needed to reach it.

**Out of scope:**
- `SubprocessController`/`AcpController`/other controller types — not
  reported affected by this incident; audit separately if desired, don't
  bundle into this fix.
- The `norespawn`/`respawn_if_done` plumbing itself — already correct and
  already used by `ShellController`; this spec's fix should reuse it where
  natural rather than build a parallel mechanism, but does not need to
  change its existing semantics.
- Anything about `ListAgents`' own cross-session discovery mechanism — it
  behaved correctly (it can only see a running Claude Code process); this
  spec's fix is what makes the process exist, not a change to how
  `ListAgents` discovers it.

## 6. Testing

- Unit test on `resync_controller`/`PersistentSubprocessController::start()`:
  a block with `agent:sessionid` set spawns eagerly on `start()` with no
  prior message queued (assert a `--resume <sid>` spawn happens, not just
  registration).
- Regression: a block with no `agent:sessionid` still registers-and-waits,
  unchanged.
- Live/manual: reproduce the incident's exact shape — restart the backend
  with an existing multi-turn agent pane open, confirm the pane resumes
  automatically (visible activity or at least a live process) with no
  message sent, and confirm `ListAgents`/an equivalent discovery mechanism
  can now see it without anyone messaging it first.
