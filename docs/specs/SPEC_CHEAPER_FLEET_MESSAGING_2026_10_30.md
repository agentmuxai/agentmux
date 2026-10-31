# Spec: cheaper fleet messaging: a bulletin that never wakes, quota mode as a setting, no-ack messages, and a count

**Status:** active, 2026-10-30. Approved the same day. The four parts were approved by the operator ("approved, lets do it", relayed by AgentY). Built in Kanban #11016, one PR per part.
**Date:** 2026-10-30
**Author:** Agent2@narko
**Verified against:** `agentmux` `main` @ `be2d8f8fe` (2026-10-30), by reading code, and against the jekt audit file and Claude session transcripts of 2026-10-29.
**Builds on:** `SPEC_JEKT_HOLD_FOR_COLD_SESSIONS_2026_10_25.md` (the hold and the notes that wait), `SPEC_JEKT_DELIVERY_STATES_AND_MAILBOX_2026_10_01.md`.

## 1. What a fleet notice costs today

One real round, 2026-10-29 20:53-21:15Z: the two leaders told the fleet to slow down for quota.

| Measure | Value | Where it comes from |
|---|---|---|
| Sends | 29 in 30 s (AgentX 10, AgentY 19), to 16 agents; 8 agents got 2-3 copies | jekt audit file |
| Size of a send | 345 characters on average (about 90 tokens), 10 K in all | same |
| Replies to the leaders | 19, 9.5 K characters in all; most carried no new fact ("ack, pausing") | same |
| What a message that starts a turn costs | the whole context is read again: 160 K to 370 K cache-read tokens per model call at the recipients measured (Clamk 158 K, Agent2 190 K, AgentY 238 K, Korp 371 K), a turn is at least one call | transcripts, `usage.cache_read_input_tokens` / assistant messages |
| Leader side | every reply that starts or joins a leader turn re-reads 240 K (AgentY): 19 replies are 19 chances to read it again | same |

The 90 tokens are not the cost. The cost is the context read each time a message starts a turn: about 48 messages (29 + 19) that each can cost 160-370 K tokens of cache reads, to move 20 K characters of content. Already handled: a message to a session cold for 55 minutes waits (hold for cold sessions). Not handled: warm or recently warm sessions, replies that say nothing, and the same text sent N times.

## 2. What other systems do (from the literature and known designs; not looked up for this report)

| Idea | Used by | Fits here as |
|---|---|---|
| **Retained message**: the broker keeps the last value per topic and gives it to a subscriber when it next connects; it wakes nobody | MQTT retained messages | The bulletin (§4): one value per key, seen at the agent's next turn |
| **Pull, not push**: a worker reads shared state when it starts a step | blackboard systems (Hearsay-II), LangGraph shared state, Kubernetes ConfigMaps | `Tasks claim` and the next-message attach read the bulletin; nothing is pushed |
| **Fire and forget**: `cast` has no reply, `call` has one | Erlang `gen_server` | `reply: none` on a message (§6) |
| **Cumulative / delayed acknowledgement**: acknowledge on the next data you send, never alone | TCP | A bare "got it" is refused; the answer rides on the next real message |
| **Artifacts, not relays**: sub-agents write results to files and pass references, so the coordinator doesn't re-read them | orchestrator-worker multi-agent systems | Already our rule for specs and reports |
| **Channel with mentions**: a room everyone may read; only a mention interrupts | Slack, Discord | The bulletin is the room, `urgent` the mention |

## 3. Decision

All four parts of the brief, in this order of build (smallest first): **no-ack (§6), the count (§7), the bulletin (§4), quota mode (§5)**. They compose: a change of quota mode posts its own bulletin, so no one broadcasts "slow down" again.

## 4. The bulletin

A post an agent sees at its next real turn. It never starts a turn and never asks for one.

- **Post:** `Message` action `bulletin`, with `text`, an optional `key` (a later post under the same key replaces the earlier: `mode`, `priorities`), optional `targets` (agent ids; default every agent of this AgentMux) and `expires_minutes` (default 1440). Posts per sender are limited (5 an hour, 20 live) and a post is at most 1000 characters.
- **Delivery:** for each running target, `blockcontroller::system_note_into_turn_or_hold(block, "bulletin:<key>", text)` (the mechanism of the cold-hold spec's §8): it joins the turn in flight, or waits and goes after the text of the next message the pane gets (`reactive::held_notes`). Nothing else changes in that path.
- **Durable board:** `held_notes` lives in memory and holds eight notes a pane. The bulletin therefore also lives in a board in srv's store (`id`, `key`, `from`, `text`, `created`, `expires`, `targets`) with a per-agent "seen" cursor. The board answers three things the memory can't: an agent that is stopped or starts later reads what it missed, a restart of srv loses nothing, and `Message action read_bulletin` lets any agent pull it.
- **Where it is attached:** the next message (as notes are), the answer to `Tasks claim`, and `Presence who`. Each shows only bulletins the agent has not seen, once.
- **How it reads.** It is another agent's words, so it is not an `[AgentMux]` note (those are srv's own, and the receivers' rules trust them). It is marked like a jekt body: `[BULLETIN:FROM=<agent> TRUST=<as for a jekt> KEY=<key> TS=<s>]` then the text, passed through the same neutralisation of marker-like text as a jekt body (`sanitize::neutralize_markers`). Receivers treat it as a jekt body (the jekt specification's section 8): information from a named agent, not an instruction from the user, never a reason to reveal or widen anything. **Info tier only:** a text that raises the sensitive flag (`is_sensitive_message`) is refused at the post, and a bulletin carries no files. Only a proven host-tier sender may post (no LAN, WAN or channel in v1).
- **Out of v1:** posting to another host's agents (bulletins stay on this AgentMux; cross-host goes by one message to the steward as today).

## 5. Quota mode, a setting

- **The setting:** `fleet:quota_mode` in the settings file: `off` (default) or `tight`, with `fleet:quota_min_priority` (default 1). A leader, or the operator, sets it through `App`/settings or a `Message` action `quota_mode` that writes it and posts the bulletin (key `mode`) with the new state. No broadcast.
- **`Tasks claim`, when `tight`:** a claim with no `id` takes only a task aimed at this agent or has `priority >= fleet:quota_min_priority`. Otherwise it answers `{claimed: false, reason: "quota mode: only current or top-priority work"}` and names how many it held back. A claim with an `id` still works (the agent was told to).
- **`Message send`, when `tight`:** a routine message (the cold-hold spec's §3 rules, which already exclude urgent, sensitive and task-naming messages) to an agent with no turn in flight is held like an idle one, `held_reason: "quota"`, and goes when the agent is next active or the mode is off. A warm agent mid-turn still gets it (it joins the turn, no new context read). Urgent and `#N` messages are never held.
- **Not in v1:** pausing schedules and jobs, a per-agent mode, a spend ceiling.

## 6. No-ack messages

The aim is that a reply that adds nothing is never sent, and the sender is told why.

- **`reply: "none"` on `Message send`/`broadcast`.** The marker gains `REPLY=none` (absent otherwise). The `Message` tool description says informational messages should carry it, and an `info` message from a leader that names no task gets it set by default (opt out with `reply: "ok"`). srv remembers `MSGID -> (sender, reply policy)` for one hour.
- **`in_reply_to: <MSGID>`** (optional) on a send. A send that answers a `REPLY=none` message is **refused**: "Not sent: that message asked for no reply. If you have a new fact, say it as its own message." Nothing is delivered, no turn starts.
- **A bare acknowledgement is refused even without `in_reply_to`:** after trimming punctuation and emoji, text of at most 24 characters that is in a short list (`ack`, `ok`, `got it`, `thanks`, `noted`, `will do`, `on it`, `done`, `+1`) to an agent that sent this agent a message in the last 30 minutes. The setting `message:refuse_bare_acks` (default on) switches it off. A wrong refusal costs one re-send with more words; a wrong delivery costs a turn.
- **The answer is a plain refusal, not a gate on who:** any agent is subject to it, the operator's own words never go through `Message`.

## 7. The count

- **What is recorded:** each delivery's audit entry gains `turn`: `started` (the pane had no turn in flight), `joined` (it joined one), `held`, `refused`. It costs one field on a line already written.
- **Per agent per day:** `Agents` action `message_stats` (read-only) sums the audit file by target agent and day: messages, turns started by a message, joined, held, refused acks, bulletins read. Cost comes from the same file's cost field when it has one, otherwise from the agent's transcript (the method of §1).
- **Before and after:** the baseline is 2026-10-29's file (sends and replies per day, per agent are in it now; `turn` starts with the first build). The report compares the week before the PR that adds `turn` with the week after the last of the four, with the same two counts: turns started by a message per agent per day, and replies per leader send.

## 8. Files

| Part | Files |
|---|---|
| No-ack | `crates/mcp/src/tool_schemas.rs` (`reply`, `in_reply_to`), `crates/common/src/api_types.rs`, `backend/reactive/sanitize.rs` (marker field), `server/reactive.rs` (the refusal), a small `backend/ack_policy.rs` |
| Count | `backend/reactive/types.rs` (`turn` on the audit entry), the delivery paths that write it, `server/app_api` (`message_stats`), `crates/mcp` |
| Bulletin | `backend/storage` (board table, cursor), `backend/bulletin.rs`, `backend/reactive/held_notes.rs` (the key prefix, nothing else), `work_claims_handlers`/Tasks claim answer, `crates/mcp` |
| Quota mode | `backend/cold_hold.rs` (the `quota` reason beside `idle`), `storage/work_board.rs` (the claim filter), settings key, `crates/mcp` |

## 9. Tests

- No-ack: a refused reply to a `REPLY=none` message, a bare ack refused only inside 30 minutes of a message from the target, a long ack-like sentence delivered, the setting off.
- Count: each delivery path writes the right `turn`, the stats sum by day.
- Bulletin: joins a running turn, waits for a pane with none, a replace by key, a stopped agent reads it on start, one agent never sees the same post twice, a sensitive text is refused, marker-like text is neutralised.
- Quota mode: a claim with no id is filtered, with an id is not, a routine send is held and released when the mode is off, urgent and `#N` messages are not held.

## 10. Open points

- The bulletin marker is new to receivers: the Global Memory entries and the jekt specification (docs site) get a line saying `[BULLETIN:` is an agent's proven words at info tier, like a jekt body. Until they do, the text is read as an ordinary note; nothing in it can act.
- Whether `Presence who` should show the bulletin (it is read often); v1 attaches it to the next message and `Tasks claim` only.
- The refusal list for bare acks is a guess from today's replies; the count (§7) shows how many it caught and whether any real answer was refused.
