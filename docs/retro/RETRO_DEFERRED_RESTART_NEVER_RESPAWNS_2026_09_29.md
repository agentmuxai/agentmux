# Retro: a deferred config restart left Korp with no process, and every jekt to it queued for an hour (2026-09-29)

**Status:** retro. The fix ships in the same PR as this retro.
**Trigger:** during the CEF 152 → 154 upgrade, AgentY (the manager) sent Korp its Windows
build go-ahead. The message was queued as "their agent is starting up, restarting or
stopping", and so were three more over the next hour. Korp's pane looked alive to the operator
the whole time.
**Impact:** the Windows build, the upgrade's critical path (3–6 h), started about an hour late.
Four messages from an automated sender sat undelivered and unreported. Nothing surfaced the
problem except the sender noticing the repeated "queued" replies.
**Host:** narko, AgentMux 0.58.2, channel `local-agent1-release-patch-bump-abe85c-306df097`,
Korp's pane `f7377bf9…`.

---

## Timeline (UTC, from that channel's `agentmuxsrv-v0.58.2.log.2026-09-29`)

| Time | Event |
|---|---|
| 06:18:35 | AgentY's first message to Korp is delivered (`deferred: false`). Korp replies at ~06:20. |
| 06:21:48 | A `ControllerResync` (a runtime-config change) reaches Korp's pane **mid-turn**. It logs "runtime-config change arrived mid-turn — deferring the restart to the end of this turn". |
| 06:22:29 | The turn ends. It logs "turn ended — applying the deferred runtime-config restart", then "persistent process kill requested" and "turn_active flip (process exited)". **No spawn follows.** |
| 06:22:58, 06:32:58, 07:00:52, 07:14:45 | Four more messages from AgentY. Each logs "delivery queued — the agent's process is starting up, restarting or stopping". `SendMessage` reports QUEUED and says "Don't resend it". |
| ~07:15 | Diagnosed from the log. The workaround: type anything into Korp's pane. |

## What happened

A runtime-config change (model, effort or permission mode) on a persistent agent is applied by
replacing the CLI process, because the flags are fixed when it spawns. If the change lands
mid-turn, `request_restart_when_idle` defers it rather than killing the turn, which was the right
fix for AgentX's lost turn on 2026-08-28. At the turn boundary, `turn_boundary_locked` sets
`restart_pending` and the stdout reader stops the process.

The design then relied on one sentence, in the reader, in `deferred_must_wait_locked` and in the
`restart_pending` field doc: **"the next message respawns it."** That's true for exactly one kind
of message:

- **A human's message** (`agentinput` → `send_message`) goes through `decide_send_action`. With no
  process and no claim, that becomes `BecomeSpawner` and starts the replacement.
- **An automated message** (jekt, MCP `SendMessage`, muxbus, the bridges) goes through
  `deliver_agent_message` → `send_user_message_outcome`. By design this path never spawns. While
  `restart_pending` is set, `deferred_must_wait_locked` returns true, so the message is queued.
- **The deferred-delivery watchdog** checks `deferred_must_wait_locked` first. While it's true, the
  watchdog resets its orphan timer every tick. It never delivers the queue and never reports it
  stranded.
- **`restart_pending` is cleared only by a spawn** (`spawn_process`).

So once a restart had been applied, the agent could be reached only by typing into its pane. Any
number of jekts waited indefinitely, and each sender was told the agent was "starting up". Nothing
was ever starting.

The restart path while idle doesn't have this problem. `resync_controller` replaces the controller,
and the new controller's `start()` eager-resumes the session right away, so an agent restarted
while idle always has a live process. Only the deferred path left the pane with none.

## Why it wasn't caught

- **The two delivery paths make opposite spawn decisions, and the restart code assumed the human
  one.** The deferred-restart work (PR #2858) came before `SPEC_JEKT_IMMEDIATE_DELIVERY_2026_09_28`
  put jekts on the queue-while-must-wait path. Each change was correct for the paths its author had
  in mind, and together they left an agent that only a human could reach.
- **"Queued" looked like success.** `SendMessage` reports QUEUED with "their AgentMux delivers it as
  soon as their agent is up. Don't resend it." That told the sender to wait, for an event that could
  never happen.
- **The watchdog's escape hatch excluded this exact state.** "Stranded" reporting exists for
  "no process and nothing starting one", but `restart_pending` was deliberately treated as "something
  in flight". That's true until the kill arm finishes, and false forever after.
- **The tests stopped at the kill.** The existing quiesce-window tests check that messages queue
  (not lost) and that the flag clears when a spawn arrives. They emulate the spawn, and nothing
  asserted that one would happen.

## The fix (same PR)

- **The restart brings its replacement up itself.** The turn-end reader calls
  `stop_for_config_restart()`, which records `config_restart_generation` in the same lock acquisition
  as the kill request. When the kill arm has finished its cleanup for that generation, it calls
  `respawn_after_config_restart()`. That eager-resumes the same session from fresh block meta, the
  same `try_eager_resume` an idle restart's `start()` uses, behind the same identity gate and
  single-instance admission check. The spawn clears `restart_pending`, and the watchdog delivers the
  queue to the new process.
- **An explicit Stop or a pane close always wins.** A restart is also a stop (`stop_pending`,
  `StopRequested`), so those flags can't tell the two apart. The new marker can. Every other stop
  request clears it in the same acquisition, and so does the shutdown path. The respawn also refuses
  if the controller no longer serves its block, or if a human message's spawn got there first.
- **If the resume is declined, the window closes anyway.** Examples: no session on file, the agent
  is live in another instance, or the credential gate refuses. `restart_pending` and `stop_pending`
  are cleared, so queued deliveries fall to the no-process rules: the watchdog reports them
  stranded after its grace window, instead of holding them forever.
- **The three comments that said "the next message respawns it" now describe the real mechanism.**

## Follow-ups (not in this PR)

1. **Make "queued" something the sender can check.** A QUEUED delivery should expire into a visible
   failure at the sender after some bound, rather than "Don't resend it" with no end.
   `log_stranded_deferred` already notes that its reporting is a log line, not a failure routed back
   to the sender. That's the same gap, from the receiving side.
2. **Show the state in the pane.** A persistent pane with no process and deliveries queued should say
   so. Korp looked alive for an hour.
3. **Review every "the next message will …" assumption against both delivery paths.** Automated
   senders are now a large share of inputs, so any lifecycle step that waits on a message has to say
   which kind.

## Workaround, for anyone on an unfixed build

Type anything into the stuck agent's pane. `send_message` spawns the replacement, and the queued
jekts are delivered right after it.
