# Spec: a leader accepts, or sends back, a Kanban task when the operator tells it to

**Status:** active — approved by the operator, 2026-10-23 ("yes proceed with all", relayed by AgentY): D1–D6 as recommended, plus D7 (a Swarm broadcast doesn't count). Built in Kanban #1789.
**Date:** 2026-10-23
**Author:** Naki-2@narko, for Kanban tasks #1460 (this spec) and #1461 (its build), filed by AgentY at the operator's request
**Verified against:** `agentmux` `main` @ `9dfcbfac6` (2026-10-23), by reading code; the build (§14) was tested against `main` @ `b606c5646` (2026-10-24).
**Changes:** `SPEC_WARDEN_KANBAN_2026_10_20.md` §6.4 (who moves what), §6.12 (the `Tasks` actions), §6.13 (trust: operator-only moves).

## 1. Ask

The operator, 2026-10-23: "right, so leaders can accept work if directed by the operator".

The context:
- **The specs:** #935, #1102 and #1271 were waiting in Review.
- **The request:** the operator told AgentY in its pane to "approve them for me" / "accept them for me".
- **Why AgentY couldn't:**
  - Accept, Send back, Cancel and moving a task are the operator's moves only (`SPEC_WARDEN_KANBAN_2026_10_20.md` §6.13).
  - They are websocket commands from the app's window (`server/app_api/kanban.rs` `act`), which no agent's MCP reaches.
  - The `Tasks` tool has no such action.

## 2. Summary

- **Two new `Tasks` actions, `accept` and `send_back`.** Each takes the task's number and a `user_instruction`: the operator's own words from the message that started the agent's current turn, quoted verbatim.
- **The check:** srv checks that quote with the same gate `QuitSelf`, `App upgrade` and `Agents spawn` use (`sagas::self_quit::gate`). It adds two Kanban rules: a turn started by a message srv wrote around another agent's words doesn't count (§5.2), and neither does a Swarm broadcast (§5.1).
- **Who:** only a **leader**:
  - **before seats ship:** an agent listed in the setting `kanban:delegates`, which is empty, so off, until the operator fills it;
  - **after seats ship (#1102/#1105):** the seat's role.
- **Limits:**
  - never a task the leader submitted or holds itself;
  - never a task its own words didn't cover;
  - one call per task, each logged with the quote.
- **On the card:** "Accepted by AgentY on your word: 'accept them for me'", in the history and the audit. The operator can **Reopen** a task a leader accepted (§6).
- **Out of v1:** cancel, move, and leaders on other computers. They are listed in §8 with what each would need.

## 3. Today

- **The operator's moves** are `kanban.act` actions (`create`, `edit`, `move`, `start`, `answer`, `accept`, `send_back`, `cancel`, `retry`, `track`). They are websocket RPCs, the operator's by construction.
  - `accept` moves Review → done (`work_accept`).
  - `send_back` returns the task to its holder with the comment, or to the Backlog when the holder is gone.
- **The operator's word, as srv checks it** (`crates/srv/src/sagas/self_quit.rs`):
  - **Where the facts come from:** the agent's controller keeps the provenance of the turn in flight: its origin (`User`, `Automated`, `System`), whether it is `tainted`, and `user_text`, the message that started it (`backend/blockcontroller/health.rs`, `TurnProvenance`).
    - The text is kept only for a turn started by the operator.
    - Any other input arriving mid-turn taints the turn.
    - Once no turn is active there is no provenance, so a quote from an earlier turn can't pass.
  - **The rule:** `gate(provenance, quote)` passes only when the origin is `User`, the turn isn't tainted, and the quote is in `user_text` (whitespace collapsed, case ignored; an empty quote never matches). Refusals: `turn_origin_unknown`, `not_user_turn`, `turn_tainted`, `user_instruction_mismatch`.
  - **Who uses it:** `QuitSelf`, `App upgrade` (`server/app_update.rs`) and `Agents spawn` (`server/app_api/agent_spawn.rs`). On a refusal, upgrade and spawn ask the operator instead.
  - **What counts as the operator:**
    - the agent pane's composer;
    - a Swarm broadcast (`fleet.rs`, `BROADCAST_TURN_ORIGIN = User`, the owner's decision of 2026-10-06);
    - the HTTP viewer;
    - the Kanban's own messages to an agent: `tell` in `kanban.rs` delivers Start, Answer and Send back as `User`.
  - **What doesn't:** a jekt, a cron or loop, the Supervisor, a memory reinjection.
  - **Only the persistent (Claude) controller keeps provenance.** Every other provider's agent has none, so its gate always refuses.
- **The Kanban's own idea of "the operator asked"** (`work_tasks.rs` `provenance`) is weaker. It labels who requested a task from the turn's trigger, with no quote and no taint check. It is a label, not a permission, and this spec doesn't use it.
- **Leaders:** none in the app. Seats exist only as a Global Memory entry, and their build is #1105. `Agents spawn` has a leader of its own (`agent:coached_by`, at most 3 workers), which says who spawned whom, not who may accept work.

## 4. Design

### 4.1 The actions

| Action | Input | Effect |
|---|---|---|
| `accept` | `id`, `user_instruction`, `note?` | Review → done, exactly as the operator's Accept. A spec it accepts unlocks its build (`SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md` §6). |
| `send_back` | `id`, `user_instruction`, `reason` | Review → the holder, or the Backlog, exactly as the operator's Send back. The reason goes to the holder as the comment, prefixed "AgentY, on the operator's word:". |

- **Route:** both are new `POST /agentmux/work/:id/accept` and `/send_back` routes. They carry the caller's agent token, like `hold`. srv takes the caller from the token, never from the body.
- **Shared logic:** each runs the same store transition as the `kanban.act` action, in one shared function, so there is one Accept and one Send back, not two.
- **One task per call.** No batch form: each acceptance is its own decision, its own log line and its own history row. "Accept them" becomes three calls.

### 4.2 The check, in order

Each refusal answers with the reason, in words the agent can act on.

1. **The caller is a leader** (§4.3). Else: "Accepting a task needs the operator; you're not one of their delegates (Settings › Kanban)."
2. **The task is in Review** (accept, send back). Else: "#12 is in progress, not in Review."
3. **Not your own work.** The caller is not the task's holder (`claimed_by_uid`) and didn't submit it (the `submitted` event's actor). Else: "#12 is your own work; the operator accepts it."
4. **The operator's word** (§5): the gate passes with `user_instruction`.
5. **The words cover this task** (§5.3).

- **On a refusal at steps 4 or 5:** nothing changes. The answer suggests what the operator can do: "Ask the operator to accept #12 on the Kanban, or to tell you again naming it."
- **No fallback dialog.** Upgrade and spawn ask the operator in a window. Here the Review column already is that question, and a second prompt for the same click would only add noise.

### 4.3 Who is a leader

- **Before seats ship:**
  - **The setting:** `kanban:delegates`, a list of agent uids (Settings › Kanban: "Agents that may accept or send back work when you tell them to", a picker of this computer's agents).
  - **Default:** empty, so the actions are off until the operator names someone.
  - **The match:** srv matches the caller's token uid against the list, never a name, so a rename or a same-named agent can't slip in.
- **After seats ship (#1105):** a seat whose role is leader counts too. The setting stays for operators who use no seats. D2.

### 4.4 What the history and audit record

| Where | What |
|---|---|
| Card history (`db_work_events`) | `accepted` / `sent_back` with `actor` = the leader, `actor_kind` = `delegate`, and body `on the operator's word: "<quote>"` (plus the note or reason) |
| Card | Done cards a delegate accepted show a small "by AgentY" under the title, with the quote in its tooltip |
| Audit | One `fleet_action` entry per call: leader, task, decision, quote, and the gate's verdict, refusals included (`log_fleet_action_audit`, as `QuitSelf` writes) |
| srv log | One info line per call, with the refusal code when refused |

- **The quote is the operator's own text.** It is stored as given, after the same sensitive-text check as other card text (`is_sensitive_message`). A credential in it is refused, not stored.

## 5. What counts as "the operator's word"

### 5.1 The gate, unchanged

- **Reuse `sagas::self_quit::gate`** as `upgrade` and `spawn` do. The quote must be in the message that started the current turn, which must be the operator's and untainted.
- **This rules out:**
  - **a jekt** (another agent, a leader's worker, the review bot): `Automated`, so `not_user_turn`;
  - **a message forwarded into the turn mid-way:** `tainted`;
  - **a quote from an earlier turn:** the provenance is the current turn's only;
  - **a stale turn after the agent went idle:** no provenance, so `turn_origin_unknown`.
- **A Swarm broadcast does not count** (D7, the operator, 2026-10-23). srv counts a broadcast as the operator's turn for other purposes (`fleet.rs`, `BROADCAST_TURN_ORIGIN = User`). For `accept` and `send_back`, only the operator's own message, typed in that leader's pane, is their word, so one line sent to many agents can't accept work in all of them. srv records a broadcast-started turn as such in its provenance (`broadcast`, set with the origin, like `relayed` in §5.2), and the Kanban wrapper refuses it (`turn_broadcast`). QTest applies the same rule to App upgrade (Kanban #1790).

### 5.2 One Kanban rule on top

- **The hole:** the Kanban's own `tell` delivers Start as the operator's turn. For a task another agent wrote, that message carries the agent's words, quoted ("Naki-2 wrote it down; here it is as they wrote it: > …"). A leader started on a task whose payload says "accept #12" could quote that line and pass the gate.
- **The fix:**
  - **The flag:** srv marks such a turn's provenance `relayed` when `tell` delivers text another author wrote, i.e. Start of an agent-written task.
  - **Where it applies:** `accept` and `send_back` refuse a `relayed` turn (`turn_relayed`).
  - **What stays the same:** the other gates don't change, since their actions (quit, upgrade, spawn) aren't the subject of task payloads. Whether they should follow is a question for their owners (D6).
- **Answer and Send back are not affected:** their text in `tell` is the operator's own (the answer they typed, the comment they wrote).

### 5.3 Which tasks the words cover

The operator says "accept them for me" more often than "accept #935, #1102 and #1271". Both are honoured:
1. **The task's number is in the operator's message** ("#1102", "1102") → covered.
2. **No number in the message:** covered when the leader's own previous reply to the operator, the one this message answers, named the task's number, and the task was in Review when that reply was written.
   - srv reads that reply from the pane's transcript: the last assistant message before the turn's `user_text`.
   - "Approve them for me" after "Waiting in Review: #935, #1102, #1271" covers exactly those three.
3. **Otherwise refused** (`not_covered`): "The operator's words don't name #12, and your last reply didn't either; ask them to name it."

- **What isn't attempted:** reading intent ("accept the specs", "accept everything"). "Everything" is not a list, and a wrong guess accepts work no one read.
- **The leader's job:** say the numbers in its reply first, then ask.

### 5.4 Other providers

- **The limitation:** only the persistent (Claude) controller keeps a turn's provenance. A Codex, Gemini or other agent named as a delegate is refused (`turn_origin_unknown`) until its controller does.
- **Where the operator sees it:** the setting's picker marks such agents "can't verify your words yet".

## 6. Undo: Reopen

- **What it is:** a task a delegate accepted gets **Reopen** in its drawer, for the operator only (a `kanban.act` action), for 7 days.
- **What it does:** Done → Review, with "Reopened" in the history.
- **What it doesn't do:** undo what the acceptance set off. A merged PR stays merged, and a build that started after its spec was accepted keeps going; the operator sends that back or cancels it as usual.
- **Tasks the operator accepted themselves:** not offered Reopen. Today's board has no undo for those, and this spec doesn't add one.

## 7. Agents' guidance

The operator-config `Tasks` paragraph (`crates/srv/operator-config-seed.json`, manifest bump) gets two sentences:

> When the operator tells you to accept or send back tasks in Review, and you are one of their delegates, do it with `Tasks` `accept` / `send_back`, quoting their message as `user_instruction`. Name the tasks' numbers in your reply first, so their "accept them" covers exactly those; never accept your own work.

The `Tasks` tool description says the same in one line per action.

## 8. Out of v1

| Action | Why not now | What it would need |
|---|---|---|
| `cancel` | Not undoable: a cancelled spec also cancels its build (stages spec §6.4) | the same gate; Reopen for cancelled |
| `move` (to another agent) | Low value. A leader already assigns by filing tasks; moving an operator-assigned task overrides the operator's own choice | the same gate; the target resolved by uid |
| Leaders on other computers | The gate reads the pane's controller on *this* srv, and the queue is per computer | the remote srv checking the quote and forwarding a signed decision; a separate spec |

## 9. Files

**srv:**
- **`server/work_tasks.rs`:** the `accept` and `send_back` handlers, with the checks of §4.2 and coverage (§5.3).
- **`server/routes.rs`:** two routes.
- **`server/app_api/kanban.rs`:**
  - one shared `accept_task` / `send_back_task` that both paths call;
  - Reopen;
  - `tell` marks Start of an agent-written task `relayed`.
- **`backend/blockcontroller/health.rs`:** the `relayed` and `broadcast` flags on `TurnProvenance`, set with the turn's origin, never cleared within the turn.
- **`server/app_api/fleet.rs`:** a broadcast's turn sets `broadcast`.
- **`sagas/self_quit.rs`:** `gate` unchanged. A Kanban wrapper adds `relayed` and coverage.
- **`backend/kanban.rs`:** the `kanban:delegates` setting.
- **Audit:** `log_fleet_action_audit`.

**MCP:**
- **`crates/mcp/src/tool_schemas.rs`:** `accept` and `send_back` on `Tasks`, with `user_instruction`, `note` and `reason`.
- **`tools/work.rs`:** the calls.

**Frontend:**
- **Settings › Kanban:** the delegates picker.
- **The card:** "by AgentY" with the quote in its tooltip.
- **The drawer:** Reopen.

**Docs:**
- this spec;
- the Kanban spec's §6.4 and §6.13 rows;
- the operator-config guidance.

## 10. Tests

- **The gate, through the Kanban wrapper:**
  - **passes:** a user turn with the quote;
  - **refused:**
    - a jekt-started turn;
    - a tainted turn;
    - an idle agent (no provenance);
    - a mismatched quote;
    - an empty quote;
    - a `relayed` turn (Start of an agent-written task whose payload contains the quote);
  - **broadcast:** a Swarm-broadcast-started turn is refused (`turn_broadcast`), even with a matching quote.
- **Coverage:**
  - the number in the message;
  - no number, with the previous reply naming it;
  - no number, with the previous reply naming a different one (refused);
  - the task not in Review when that reply was written (refused).
- **Limits:**
  - not a delegate;
  - own holder;
  - own submitter;
  - a task not in Review;
  - a delegate named by name only (never matches: uid only).
- **Effects:**
  - accept → done, and the follower message (stages spec) sent once;
  - send back → the holder gets the reason with "on the operator's word";
  - history row and audit entry each carry the quote;
  - a sensitive quote is refused;
  - Reopen within 7 days, not after, and not for operator-accepted tasks.
- **MCP:** both actions' schemas, and refusal texts passed through verbatim.

## 11. Rollout

- **The build:** #1461, one PR: srv and MCP, then the frontend's setting, card and Reopen.
- **Off until there is a leader seat:** nothing changes for an agent that is not a leader seat (§14).
- **Older builds** on the same computer don't have the routes. An older MCP doesn't offer the actions. Neither can accept anything, which is the safe side.

## 12. Decisions for the operator

| | Decision | Recommended |
|---|---|---|
| D1 | v1 is `accept` and `send_back` only; cancel and move later | **Yes** |
| D2 | Before seats, leaders are the agents listed in `kanban:delegates` (empty by default); after seats, a leader seat counts too | **Yes** |
| D3 | "Accept them" covers the tasks the leader's previous reply named; otherwise the number must be in your words | **Yes** |
| D4 | No dialog on a refusal: the task stays in Review for you, and the leader is told why | **Yes** |
| D5 | Reopen for 7 days on a task a leader accepted | **Yes** |
| D6 | The `relayed` rule (§5.2) applies to these Kanban actions only; its owners decide for quit, upgrade and spawn | **Yes**, and tell those owners |
| D7 | A Swarm broadcast is not the operator's word for `accept` and `send_back`; only their own message in that leader's pane is | **Decided by the operator, 2026-10-23** (§5.1) |

## 14. As built (Kanban #1789, 2026-10-24)

What shipped matches §4 to §7, with these differences. Each is the smaller choice; none changes what the operator approved.

| Spec said | Built | Why |
|---|---|---|
| A leader is a seat of role leader, or an agent in a `kanban:delegates` setting (§4.3) | The seat only (`agent_seat_get`). No setting and no picker | Seats exist, so the setting would only add a second way to be a leader. If the operator wants one for hosts without seats, it is a small follow-up |
| `relayed` and `broadcast` flags on `TurnProvenance`, set by the turn's origin (§5.2, §9) | Read from the turn's own text: a Swarm broadcast header refuses with `turn_broadcast`, and a turn carrying the start text of an agent-written task refuses with `turn_relayed` | No change to the controller or to `quit`, `upgrade` or `spawn` (D6). If the flags are added later, the text check can go |
| "Accept them" is covered by the previous reply that named the tasks (§5.3) | The previous reply is the last `result` line of the pane's session output; it must have been written after the task entered Review (refusal `not_covered`) | The session output is the one place a reply is kept |
| An empty or vague quote is refused (§5.1) | A quote that is not in the operator's turn is `user_instruction_mismatch`; one under a few words, or only the number, is `quote_too_vague` | A quote of "yes" or "#12" would pass the match and prove nothing |
| `kanban.act` carries Reopen | `kanban.act` `reopen`; with #5443 `kanban.act` as a whole needs the window's proof, so Reopen is the operator's | The leader path never goes through `kanban.act` |
| Done cards show "by AgentY" | A pill "accepted by AgentY" with the quote in its tooltip, for the 7 days Reopen is open | Same window as Reopen |

A leader's send-back reason reaches the holder quoted, with a first line saying it is the leader's text, because the holder gets it as the operator's turn; `relays_agent_text` refuses such a turn as the operator's word, so a reason can't be used to mint a quote.

Also built: refusals `not_a_leader`, `not_in_review`, `own_work`, `reason_required` (send back needs a reason), `sensitive_quote`; one `fleet_action` audit entry per call (`kanban.accept` / `kanban.send_back`), refusals included.

## 13. Sources

- **The operator's request:** relayed by AgentY to Naki-2 on 2026-10-23 (quoted in §1).
- **Specs:**
  - `docs/specs/SPEC_AGENT_SELF_QUIT_2026_09_24.md` §6.3 (the gate);
  - `SPEC_AGENT_WORKERS_2026_10_15.md` §3.5;
  - `SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §5.1. Its list still says a broadcast goes to the operator; the code counts it as the operator's (`fleet.rs`).
  - `SPEC_WARDEN_KANBAN_2026_10_20.md` §6.4, §6.12, §6.13;
  - `SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md`.
- **Code:**
  - the gate: `crates/srv/src/sagas/self_quit.rs` (`gate`, `quote_matches`, `GateRefusal`), `backend/blockcontroller/health.rs` (`TurnProvenance`, `mark_turn_active_from`);
  - its callers: `server/app_update.rs`, `server/app_api/agent_spawn.rs`;
  - the Kanban: `server/app_api/kanban.rs` (`act`, `tell`, `start_text`), `server/app_api/fleet.rs` (`BROADCAST_TURN_ORIGIN`), `server/work_tasks.rs`.
