# Spec: a leader is told when an agent that owes it work has gone quiet, in one grouped note

**Date:** 2026-11-21
**Status:** active — approved 2026-11-21 (task #48287; the build is #48288) with D1 as written (30 minutes), D2 off and D3 yes, and one change (§7: a worker's turn ending without a report is no longer a note by itself). Phase 1 is built (§13); phases 2 to 4 are not.
**Author:** Loap-3@narko
**Area:** agent lifecycle
**Extends:** [`SPEC_AGENT_WORKERS_2026_10_15.md`](SPEC_AGENT_WORKERS_2026_10_15.md) §3.3 (worker notes, `backend/worker_notes.rs`).
**Related:** [`SPEC_SWARM_SUPERVISOR_2026_10_16.md`](SPEC_SWARM_SUPERVISOR_2026_10_16.md) (the turn-end judge; §6), [`SPEC_AGENT_AVAILABILITY_EVERY_PATH_ONE_DECISION_2026_10_31.md`](SPEC_AGENT_AVAILABILITY_EVERY_PATH_ONE_DECISION_2026_10_31.md) (the statuses this reads).

## 1. The problem

A leader hands work to an agent and expects to hear when it is done or stuck. Today it hears only about **spawned workers** (`agent:worker_of`): a turn that ended without a report, a question left waiting, a failure, an exit. An agent that holds a leader's work but was not spawned by it (a seat lead's agent, the holder of a task the leader filed) gets no such note, and no note covers an agent that simply stops: it ended a turn with a long-running command left in the background, waiting for an event that command never sends, and nothing in AgentMux looked at it again. The leader found out by chance, after a long time.

Two facts make this cheap to fix without a polling loop that spends tokens:

- srv already knows, in memory, when each agent last wrote output (`ctrl.last_output_ms()`, behind `live_facts`' `TurnFacts.idle_ms`), whether a turn is running, which Kanban tasks it holds (`live_facts::claimed_tasks`), the status line it set (`continuity_facts`), and each task's filer (`WorkItem.created_by_uid`). Nothing new has to be asked of a model or of the agent.
- The note machinery exists: a broker observer, per-worker throttling (`Gaps`), delivery into the leader's turn or held for its next (`deliver`, `tell_leader_held`, `cold_hold`), and a lead lookup that is not limited to spawned workers (`leader_pane_of`, from the seat).

The requirement: **a leader of an agent that owes it work is told, once and in the same note as everything else about its agents, when that agent has gone quiet, and nothing is sent while all is well.**

## 2. What "owes work" and "quiet" mean

An agent is **watched** only while it owes work, so a finished agent with an empty backlog is never pinged and an agent a person talks to directly is not watched on its own account.

| Owes work when it | Counted |
|---|---|
| holds a claimed Kanban task that is not on hold and is not waiting on a question | phase 1 |
| is a spawned worker that has not reported to its leader since it was last given work (its first message, or any message from the leader): it **owes a report** | phase 1. A worker that ends a turn without reporting is therefore quiet after the quiet time, which is what replaces the note at every such turn end (§7) |
| was sent an urgent message that names a task (`#N`) and has not answered it, by a message to the sender, an update of that task, or its completion | phase 3 |

An owing agent is **quiet** when one of these holds for the quiet time (default 15 minutes):

| | Condition | Why it is quiet |
|---|---|---|
| **Q1** | no turn is running and its last output is older than the quiet time | it ended a turn and nothing started another: the case in §1 |
| **Q2** | a turn is running and has written nothing for the *running* quiet time (default **30 minutes**, its own setting) | a hung tool or a wedged turn |

- **Why Q2 is longer.** A foreground build or test run can write nothing for a quarter of an hour and be healthy; telling a leader about it every time would teach leaders to ignore the note. The 30 is a recommendation; the operator can set both to 15 (D1).
- **Not quiet, whatever the clock says:** a turn asking the user a question (`Waiting`: today's "asked" note covers it); a task on hold; a task waiting on the operator's answer; an agent that set `Presence status waiting` and has not passed its time (§5); an agent that cannot run (below).
- **Cannot run is not quiet, it is told at once.** `Limited`, `SignedOut`, `Stopped` and `Error` (`agent_state::AgentState`) with work owed are reported to the watchers immediately, as a worker's failure and exit are today, now for every watched agent and not only spawned ones. The agent's own nudge (§4) is pointless for them.
- **A background command still running when a turn ends** counts as quiet only after the quiet time, not at once. AgentMux cannot tell a server that will never send the awaited event from a build that will finish: a rule keyed on the command's name would be wrong both ways. What it can do is put the commands left running (`continuity_facts` already lists them) into the nudge, so the agent that forgot them sees them (§4).

## 3. Who is told

A watched agent's **watchers** are the union of:

1. its **seat lead** on this machine with a pane open (`leader_pane_of`; for a spawned worker this is its leader, as today);
2. the **filer** of a task it holds, when the filer is an agent with a pane open on this machine and not the agent itself (`created_by_uid`). A task a person filed from the Kanban has no agent to tell: §6 covers what the person sees;
3. (phase 3) the **sender** of an unanswered urgent message naming a task.

One agent can have two watchers (its seat lead and a task's filer). **Each hears the same facts, in its own note**, each once: the lead that did not file the task still needs to know its agent stopped. A watcher who is the agent itself is dropped. A watcher that is another host's agent is §8.

## 4. When, and who first

Nothing is sent while the agent moves. The timeline for one quiet episode, from the moment of the agent's last activity (T):

| Time | What happens | Cost |
|---|---|---|
| T + quiet time | **AgentMux nudges the quiet agent once** (Q1 only; a Q2 turn cannot read it): a system turn, `trigger.kind: system`, saying: "You hold #N and nothing has run for 15 minutes. Commands still running: …. Carry on, or say what you are waiting on." | one turn of the quiet agent: the point |
| T + quiet time + 5 min | still no activity: the watchers are told, in the grouped note (§7) | one line in a note |
| agent cannot run | the watchers are told at once, no nudge | one line |
| after any note | no further note about that agent for an hour, until it moves | none |

- **A nudge is allowed once an hour.** An agent that answers it ("ok") and goes quiet again within the hour would otherwise be nudged every 15 minutes while its leader hears nothing: a quiet episode that begins less than an hour after a nudge skips the nudge and goes to the watchers after the quiet time.
- **The nudge is no one's report.** It is a system turn: `worker_notes::parse_event` already ignores system turns as reports, so it neither silences the leader's note nor starts a "finished a turn without reporting" note. What the agent then writes is ordinary output and counts as activity (§5).
- **Q2 has no nudge:** the leader is told after the running quiet time plus 5 minutes (the turn may still be healthy; the leader can look at the pane).
- **No model call, no poll of the agent.** A sweep every 30 seconds reads in-memory state for the agents that owe work, and the claimed-task list once per sweep (one SQLite read, as `live_facts` does for the Swarm). Nothing runs for an agent with nothing owed.

## 5. What resets the timer

The clock is **the agent's own activity**, one clock per agent (not per watcher), so what resets it must be something only the agent does:

| Resets the clock | Why |
|---|---|
| output written by a turn (`last_output_ms`) | it is working |
| a message it sends to anyone | it is moving; what it says there is not this watch's business. A message to a watcher is also that watcher's *report* (the existing rule: no "ended a turn without reporting" note for a turn in which the worker messaged its leader) |
| an update, heartbeat or completion of a task it holds | the task moved |
| `Presence status` with state `waiting` (stops the clock until its time, below) or any new status line | it said what it is doing |

**Does not reset:** a message *to* the agent, including AgentMux's nudge and the leader's note's follow-ups (otherwise a leader's ping would hide the very silence being watched); the agent's own pane being opened or scrolled; a leader reading it.

**Muting.** An agent that knows it will be quiet says so: `Presence status` with state `waiting` already exists; it gains an optional `until_minutes` (at most 240). With `waiting` and no `until_minutes` the clock is stopped for 30 minutes from `set_at`, then runs again (a status line left up for hours is the thing this spec exists to catch). A leader can mute one agent for a stated time ("building, back in 40 minutes") with a new `Agents` action `mute_quiet` (agent, minutes up to 240, a reason shown in the Swarm). Both are recorded as who did them, never as the operator.

## 6. Where a person sees it

The same source feeds both places; no modal, no new pane (the confirm-panel rule does not apply: nothing asks anything).

- **Swarm row:** a muted "quiet 22 min" after the agent's state, from a new `quiet` group in `LiveFacts` (`since_ms`, the tasks owed, the watchers), so `Agents status` and the Swarm read one fact. The row's own state (idle, working) is unchanged.
- **Kanban card:** the same "quiet 22 min" on a claimed card, from the same group.
- **A task a person filed:** no agent watches it, but the Swarm row and card show the quiet time as above, and the Swarm ranks the agent "needs you" only if the operator turns on D2 (off by default: a person is not paged by a timer).

This is a visual change (a text chip in the Swarm and the Kanban): it needs the macOS and Linux look checks before merge.

## 7. One grouped note

A leader gets **one note listing every agent of its that needs a look**, each with its reason, replacing separate notes that arrive within the same window:

```
[AgentMux] 3 of your agents need a look:
- Agent3: quiet 22 min on #1234 (idle; commands still running: dev server)
- Agent5: finished a turn without reporting. Its last words: "next I run the check"
- Agent7: stopped: its process exited
```

- **Reasons** (one line each, the existing texts kept): quiet, finished a turn without reporting, waiting for the user, failed (with the failure's title), exited, cannot run (limited, signed out), account near its limit.
- **Mechanism.** A per-leader outbox: an event for a watched agent is queued under (leader, agent) and flushed 20 seconds after the first, or at once for a failure or exit when the window is empty. A later event for the same agent replaces its line. A flush with one line is today's single-line note, unchanged. The outbox delivers through the existing `deliver` and `tell_leader_held`, so a cold leader still gets it at its next turn and a hot one inside its running turn.
- **The existing per-worker notes** (asked, failed, exited) become reasons in this note: same facts, same texts, same one-a-minute throttle per agent (`Gaps`), now for every watched agent and not only spawned workers.
- **Approved change: a turn that ends without a report is no longer a note by itself.** Today's note fires at every such turn end, also when the worker is only waiting on a build, and each one starts a leader's turn for nothing. A leader is told when the agent has been quiet for the quiet time plus the grace (§4), fails, exits, cannot run, or waits on a question. The last words of the turn that ended may be quoted in the quiet line ("quiet 22 min after a turn that ended: 'next I run the check'"), so what the leader reads is no poorer than the old note's. A turn in which the worker messaged its leader was never a note, and still is not a quiet-maker: the message is a report and resets the clock (§5).
- **The usage notes** ([#7872](https://github.com/agentmuxai/agentmux/pull/7872): an account at 75% and at 90%) are about an account, not an agent, and already go once per lead. They are **folded in as lines under "accounts"** when they fall in the same window and otherwise sent as now. (D3 asks whether to fold them at all; the build can ship without it.)
- **Held-note rule.** A "quiet" line is information that asks nothing, so it may wait for a leader's next turn; a failure, an exit and a waiting question are delivered at once, as now.

## 8. Other machines

What a LAN peer reports about its agents today (`swarm_remote::RemoteAgent`): state, summary line, provider, seat, account and where it runs. **Not** their held tasks, last output time or idle time, which is what quiet needs.

- **An agent here whose lead is on another machine** (`Name@host`; `is_remote_lead`): today `tell_leader` returns false and nobody is told. Quiet is computed here, where the agent runs, so this host can tell it; what is missing is a way to deliver a system note to an agent on another host. The route a jekt takes over the LAN exists; **phase 4** (section 19) sends the note through it, signed as this host's AgentMux, and nothing is sent when the lead's host is not reachable (the note is dropped, not queued for a day: it is about a moment).
- **An agent on another machine whose lead is here:** that machine computes quiet and sends the note the same way. Until then the Swarm's other-host rows show only the state the peer reports, as today.
- **The first build covers none of this.** Phases 1 to 3 are one machine.

## 9. Settings

| Setting | Default | |
|---|---|---|
| `agents:quietminutes` | 15 | the quiet time for Q1; 0 turns the whole feature off |
| `agents:quietrunningminutes` | 30 | the quiet time for Q2 |
| `agents:quietescalateminutes` | 5 | how long after a nudge the watchers are told |

All three live in the settings schema and the generated template; a change applies to the next sweep. The 1-hour spacing and the 20-second window are constants, not settings.

## 10. The Supervisor, and what this is not

[`SPEC_SWARM_SUPERVISOR_2026_10_16.md`](SPEC_SWARM_SUPERVISOR_2026_10_16.md) acts when a **turn ends**, for panes that opted in, and asks a judge model. This acts when **nothing happens**, for agents that owe work, with no model. They coexist:

- a Supervisor continuation is output, so it resets the clock; a Supervisor "paused, needs the user" is a waiting agent, not a quiet one;
- the nudge is not a Supervisor continuation and is not counted against its limit of five in a row.

Not in this spec: restarting or stopping a quiet agent (the leader decides), a judgement of whether the work is going well, and any alert to a person's device.

## 11. Phases (each one PR)

1. **The detector and the note for every watched agent.** The signals of §5, Q1 and Q2, the nudge, watchers from the seat lead and a held task's filer (§3 rows 1 and 2), the settings (§9), and the existing note events generalised from "spawned worker" to "watched agent" (§7's third bullet), and the every-turn-end note for a worker removed in favour of quiet (§7's approved change). Tests: the quiet and not-quiet table above, one case per row; the timeline of §4 with a fake clock; no nudge twice in an hour; the nudge is not a report; a message to the agent does not reset it; a second watcher is told too; a worker's turn that ends without a report sends nothing at its end, and the leader is told at the quiet time plus the grace with the turn's last words quoted; a worker that reported sends nothing; failed, exited and asked still go at once.
2. **One grouped note and the display.** The per-leader outbox (§7), folding the existing reasons and, if D3 says so, the usage notes; the `quiet` live-facts group; the Swarm chip and the Kanban chip (look checks on macOS and Linux).
3. **Urgent-message watch, and muting.** Watchers row 3 and its owed work; `Presence status` `until_minutes`; `Agents mute_quiet`.
4. **Another machine** (§8): the LAN delivery of the grouped note, both directions.

## 12. What is not settled

**Decided at approval (2026-11-21):** D1 as written (a running turn that writes nothing is quiet after 30 minutes); D2 off; D3 the usage notes fold in when they arrive in the same window. The rest below is for the build to settle.

- **D1** — Q2 at 30 minutes, not 15 (§2). The operator's number was 15 for the case in §1, which is Q1; Q2 is the cautious reading.
- **D2** — whether an agent quiet on a task a person filed should rank "needs you" in the Swarm (§6). Off by default.
- **D3** — whether the usage notes are folded in (§7), or left alone.
- **A watched agent whose pane is closed** (parked, `agent:parked`): the nudge needs a controller; a parked agent still has one if it is running. The detector reads the same state either way; whether a parked agent should be watched at all is the build's first question, answered from `availability_view` (a stopped, parked agent is "cannot run", not quiet).
- **Whether `last_output_ms` moves for every provider** (Claude, Codex, ACP harnesses, container exec). It is the signal for Q1 and Q2; the build verifies it per controller before turning the feature on for that kind, and an agent whose controller cannot say is never called quiet (it is reported only through the events it already produces).
- **The exact Kanban states** for "on hold" and "waiting on a question" on a claimed card: the build reads them from the board's own fields, not from this table's words.

## 13. Phase 1 as built (task #48288)

`backend/quiet_watch.rs` holds the decision (`Watch::step`, pure, tested with a fake clock) and the sweep; `backend/worker_notes.rs` keeps the event notes. Where the build differs from, or leaves for a later phase, what is written above:

- **Clock.** One clock per agent: the newest of its output (`last_output_ms`), the last message it sent to anyone (from the delivery record `worker_notes` keeps) and the last update of a task it holds. An agent first seen owing work, and one found again after srv restarted, starts with a whole quiet time, so an upgrade does not make every agent quiet at once. A message *to* the agent does not move it.
- **Owes a report.** A spawned worker owes one while its leader has messaged it more recently than it has messaged the leader. A worker given its first brief by the spawn itself (not a message) starts owing at its leader's first message.
- **Called quiet only when it can be.** The agent is working or idle, and its controller has a turn tracker. A question waiting for the user, an agent that cannot run (stopped, failed, limited, signed out) and a controller that cannot say are never called quiet; the first is told by the asked note, the next by the failed and exited notes and the account notes that exist, so no note is added for them.
- **Notes now cover any watched agent.** The failed, exited and asked notes, which were for spawned workers, are sent for every agent that holds a claimed task too, to the same watchers (§3 rows 1 and 2); the text says "Your agent" for one that is not a spawned worker. A turn that ends with a question for the leader and no report is still a note (the worker waits for the answer); a turn that ends any other way is not.
- **Not in phase 1.** The status line as something that resets the clock and `Presence status waiting` as a mute (phase 3, with `until_minutes`); the commands still running in the nudge text (the list is not read by srv yet); the grouped note, the `quiet` live-facts group and the chips (phase 2). Each quiet note is one note to each watcher.
- **Settings.** `agents:quietminutes` 15 (0 turns the watch off), `agents:quietrunningminutes` 30, `agents:quietescalateminutes` 5.

## 14. Phase 2, the data side, as built (task #48288)

The grouped note and the `quiet` group are built; the Swarm and Kanban chips (the visible half of §6) follow in their own PR with the macOS and Linux look checks.

- **`backend/note_outbox.rs`** holds the per-leader outbox (§7). `Outbox` is pure and takes the time as an argument: the first note that can wait opens a leader's window; whatever else arrives for that leader within 20 seconds joins it, and a later note about the same agent replaces that agent's line. A note that must be heard now (a failure, an exit, a question) goes out at once when the leader has no window open, so a lone failure is never delayed; one that arrives inside an open window leaves with it. `worker_notes::install`'s two-second tick delivers the windows that have run their time.
- **One line is the old note.** A window of one line sends today's text and today's wait key. Several lines send `[AgentMux] N of your agents need a look:` and a line each, the existing texts kept. A group waits for a cold leader's next turn only when every line can, under a key of its own (`group:<leader pane>:<opened>`), so a later group never replaces an earlier one's lines. A group with any line that must be heard is delivered at once, with the held lines riding along.
- **Callers.** The failed, exited and asked notes and the quiet note all queue through it (`worker_notes::note_line_for` is `note_for` before it is addressed). The one-a-minute gap per agent still applies before a note is queued.
- **Not in this build.** Folding the 75% and 90% usage notes in (D3): they still go on their own, once per lead, as before. The spec allows shipping without it.
- **`quiet` live-facts group** (`since_ms`, `quiet_ms`, `tasks`, `watchers`), asked for by name or in the `facts` and `full` details, read from the last sweep (at most 30 seconds old). An agent is shown as quiet from the quiet time (the running time for a turn that is running), whether or not it was nudged or its watchers told. `Agents status` adds `quiet 22m` after the state.

## 15. Phase 2, the chips, as built (task #48288)

- **One source.** srv's last quiet sweep (`quiet_watch::quiet_view`) feeds the `quiet` live-facts group (§14), the runtime row the Swarm reads (`quiet_since_ms`) and the holder the Kanban card carries (`holder.quiet_since_ms`), so the three show the same fact.
- **Swarm row:** a muted "quiet 22 min" after the status chip, in the row's own note style, counting up with the row's tick; the row's state is unchanged. Hover says the agent holds work and has done nothing for that long.
- **Kanban card:** the same "quiet 22 min" as a pill on a claimed card whose holder is quiet, counted from the card's own clock.
- **Lag.** Both read the runtime store's refresh (every 20 seconds) after the sweep (every 30), so a chip can appear up to about a minute after the quiet time and clear about as late after the agent moves.
- Text through `swarm.quiet.*` and `kanban.quiet.*`. No new pane, no modal, nothing asks anything.


## 16. Phase 3, muting, as built (task #48288)

- **Waiting with a time.** `Presence status` takes `until_minutes` (1 to 240) with state `waiting`; srv keeps the end time in the line's pane meta (`term:live_phrase_until`). While it is in force the sweep counts the agent as active now, and when it ends the quiet clock runs again from the end. A `waiting` line with no time stops it for 30 minutes from when it was set, and a longer time, however it arrives, is cut to 240.
- **Any new status line is activity,** at the time it was set. A line that srv wrote for a task claim counts the same, which the claim already did.
- **A leader's mute.** `Agents mute_quiet` (`POST /api/v1/agent/mute-quiet`): `agent`, `minutes` (1 to 240; 0 lifts it) and a short `reason`. Only the agent's lead may: the lead of its seat, which for a spawned worker is the agent that spawned it. Not the agent itself (it has the status line), and not without an agent token. It is recorded in the audit log as the calling agent. The mute is kept in memory, by agent, until its end; a restart of srv gives the agent a whole quiet time, which is what a mute of at most four hours would give anyway.
- **Not in this build.** The Swarm does not show a mute or its reason yet; the urgent-message watch (§2, third row) and the other-machine case (§8) are the next phases.

## 17. Phase 3, the urgent-message watch, as built (task #48288)

- **What it records.** When an urgent message that names a task (`#N`) is delivered to an agent's pane, srv keeps the sender's name, the recipient's pane, the task numbers the message named and the time. Nothing else: not the text, not the files, not the subject. At most 256 are kept, for a day at most.
- **What it refuses.** A message is kept only when its sender was proven for that message (`host-verified`, `channel-verified`, `lan-verified` or `wan-verified`, the same trust the marker carries, computed by the same function). A message from a sender that is `self-declared`, `network-claimed` or `unverified`, one that is not urgent, one that names no task, and one that was not delivered, leaves no record.
- **It cannot change a delivery.** The record is made after the delivery is decided, on a copy of what it needs. It never waits: if the list is in use at that moment the ask is dropped. A panic in it is caught. Delivery returns exactly what it would have returned without it.
- **What the watch does with it.** At each sweep an ask counts as owed work for the recipient while a task it named still waits on the recipient, as long as the recipient has not messaged the sender since, and has not updated one of those tasks since. The recipient is nudged like an agent that holds a task ("You have an unanswered urgent message about #N ..."), and the sender is told, as the filer of a task is, in the grouped note ("... without answering your urgent message about #N"). An ask that names no task waiting on the recipient is forgotten at the first sweep.
- **Tests.** One for each of the three limits above (what is kept, whose messages, delivery unchanged), the answers that settle an ask, the wording of the nudge and the note, and the path from a delivered message to the recipient owing it.

## 18. Fewer notes for an agent that waits on purpose, as built (tasks #48605 and #48606)

- **A command of its own running.** An agent whose turn has ended with a command or job it started still running in the background is waiting on it, not stuck. It is not nudged, its quiet time is four times the usual one (an hour by default), and the Swarm and Kanban do not show it as quiet before then. When its watchers are told, the note names the command: "Its turn ended, and a background command is still running: <the command>" (one line, cut; with "(and N more)" for several). A turn that is still running is described as before.
- **Told once for a quiet.** The same agent is not told again until it moves (any output, message or task update starts a new quiet); the hourly repeat is gone.
- **Holding while quota mode is tight.** While quota mode is tight, an agent whose current status line is its own `waiting` line (not a line srv wrote for a task claim) has its quiet clock stopped for as long as that line stays and the mode stays tight. Outside quota mode the `waiting` line keeps its time limit (§16). Tasks that are on hold are not owed work and were never watched.
- **Urgent messages (§17), two corrections.** Each sender of an urgent message about a task is kept (two senders asking about one task are both told), and an incoming ask is not counted as the recipient's own activity.
- **Not changed here.** How a message to a container agent is held while it runs a command is delivery, not this watch.

## 19. Phase 4, another machine, as built (task #48288)

Only the first case of section 8 is built: **a worker here whose lead is on another machine** (`Name@machine`). The other direction needs nothing more, since the other machine runs the same code and sends to this one. The Swarm's rows for other machines' agents show no quiet chip: a peer reports no more than it did.

- **What is sent, and from where.** Quiet is computed on the worker's machine, from its own state only; nothing is read from the lead's machine. When the watch would tell the lead, it queues one note per quiet (told once, as in section 18) and srv sends it over the LAN jekt route, signed by this machine's own sender key (the key a Swarm broadcast uses), to the one LAN peer of that name that lists the lead. The note is not queued for later: a peer that does not answer loses it. It is not grouped with the lead's other notes (one per quiet); the limit below keeps a flood in check.
- **a. How it arrives.** On the lead's machine the note is a message from the sending machine's own AgentMux, with its own marker and trust line: a `[BULLETIN:FROM=<sender> TRUST=lan-verified INSTANCE_STATUS=approved TS=<time>]` block, then the words, then `[/BULLETIN]`. It is never an `[AgentMux]` note and never carries the local `[AGENTMUX-QUOTED]` prefix. The words are written on the lead's machine from the fields below; text from the sender is not put into the block as it came. Marker words in every field the sender supplies (the agent's name, the last line, the machine's name) are quoted.
- **b. Closed fields.** The agent's name, a state word (`idle` or `running`), the minutes quiet, the task numbers (at most eight) and at most 120 characters of the last line, flattened to one line and quoted. A text with any other field, or a state that is neither word, is not a note. No paths, no message bodies, nothing else.
- **c. Who is believed.** The lead's machine accepts a note only when: the signature verified (`TRUST=lan-verified`); the install that vouches for the sender key is one this machine trusts (approved, not new and not revoked: its certificate verifies for exactly that key, and the install is the one the directory or a pin trusts); the sender is one LAN peer here, found by its sender name, and that peer lists the named worker; and the lead is a running agent here. A note about another machine's agent is refused, whoever sent it. A note that was taken is counted against its message id like any signed jekt, so a copy of it does not verify again. The answer to the sender says only that it was taken or refused.
- **d. A limit, and a turn.** At most four notes an hour are shown per lead per sending machine; the rest are replaced by one held line (`N more quiet notes from <machine> in the last hour, not shown here.`) that is kept up to date. A note, like every quiet note here, **waits for the lead's next turn and never starts one**: it joins a turn that is running, else is held. The local and the remote notes behave the same.
- **e. Where it is not sent.** Nothing is sent when the lead's machine is not exactly one LAN peer here that lists the lead, or when this machine has not pinned the lead's key. The drop is logged once for that lead and reason, not at every quiet.
- **A change that comes with it.** This machine's sender key (its own sender name, with the key stored for it, and no other name) is now answered with the same install certificate as an agent's key, so a peer that trusts the install pins it at first contact, as it does an agent's, instead of waiting for the person to accept it. A Swarm broadcast to another machine benefits too.
- **Tests.** The note's fields (closed, cut, cleaned again on arrival), the block (own marker, no `[AgentMux]` note, marker words in every field quoted), the limit (four, the folded line, per lead and per machine, an hour later), who is believed (one refusal per condition above, and every other jekt unchanged), the choice of peer on the sending side, and the queue. The path between two machines is checked on a testbed host with its custodian told first.
