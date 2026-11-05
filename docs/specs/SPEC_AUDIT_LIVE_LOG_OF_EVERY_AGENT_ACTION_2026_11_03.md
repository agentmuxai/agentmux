# Spec: the Audit as a live log of every agent action

**Status:** active — approved by the operator 2026-11-03 (relayed by AgentY: "3 specs look good, go with recommendations"), so every open question in section 5 takes its recommendation. Task #21665; the build is #21666, in five PRs (section 4). PR 0 is the first.
**Author:** Loap@narko, 2026-11-03.
**Asked by the operator (2026-11-03):** "i often have the situation where there is a lot of activity, I hear the activity sounds, but I dont know where they are coming from. I thought that would be in the audit, which should show every agent action, right? we would want a live log of everything, we also want healthy use of links so we can directly access agents and even messages."
**Related:** `crates/srv/src/backend/audit_query.rs`, `frontend/app/view/audit/`, `frontend/app/notification/sound/`, `SPEC_WARDEN_SPLIT_INTO_VAULT_HISTORY_AUDIT_2026_10_28.md` §4, the "Links open in AgentMux" task #14995 (`agentmux://task/N`).

## 1. In one paragraph

The Audit shows what was *sent* (jekts, MuxAPI actions, registrations, agent starts and ends). It does not show what agents *do*, and it checks for news every 5 seconds instead of showing it as it happens. This spec adds, in three PRs: rows for tool calls and turns, taken from the agent output srv already writes; a live tail pushed by srv; and, for sounds, a "who was that" answer: the row that made a sound lights up, and a "last sound" item names the agent and opens its row. Every row names its agent, message, task and PR as links.

## 2. What exists (read on `origin/main`, 2026-11-03)

| Piece | Fact |
|---|---|
| Rows today | `audit.query` merges the jekt audit file (`<shared>/audit/jekt-audit.jsonl`, append-only, never rotated, read from its end) and the agent segment ledger. Kinds recorded: `jekt`, `action`, `registry`, `agent_start`, `agent_end`. The kinds `tool` and `thought` are declared but answer "not recorded" (`audit_query.rs`). |
| Message text | Not kept. A jekt line holds a hash and a length. |
| Row links | `AuditLinks`: `history` (definition id, zone, `at_ms`, optional `at_byte`) and `pane` (block id). No task, PR or message link. |
| Live | None. `audit-manager.tsx` calls `audit.query` every `WARDEN_REFRESH_MS` (5 s) while you are at the newest page. No audit event exists in `mps-events.ts`. |
| Where tool calls are | In the agent's transcript. Every output line of every agent srv runs is appended in one place, `transcript/append.rs::append_output_line`, to the block's `output` and to the global transcript store (zone `agent:<id>:current`). The stream-json lines there hold every `tool_use`, thinking block and `result`. Nothing indexes them for the Audit. |
| Sounds | All in `sound-service.ts`: six event sounds (`agent.turn.complete`, `.turn.error`, `.turn.interrupted`, `.message.accepted`, `.message.rejected`, `.waiting.for.input`), one tone per tool call (`tool-started`), the looping waiting tone, and the shutdown-pending tone. Every one but the last is triggered with the pane's `blockId` in hand. Two audio entry points: `SoundPlayer.play` (event sounds) and `ToolTonesPlayer.play` (tool tones). |

So the data for "everything" is already written; the Audit has to *index* it, not record it twice.

## 3. Design

### 3.1 Live

- srv publishes a broker event `AuditEntry` carrying the new row (the same `AuditEntry` shape `audit.query` returns), whenever a row is created. The view subscribes; the 5 s poll stays only as the catch-up after a reconnect (`audit.query` with `since_ms` = the newest row held).
- The view is a **live tail**: at the newest row it follows new ones; once you scroll back it stops following and a pill says "N new" (jump to newest). A Pause button freezes the list and counts the same way. Filters (agent, kinds, text) apply to live rows too.
- Bursts (a tool-heavy agent can make 10 calls a second) are batched: one event per 100 ms carrying the rows of that window.

### 3.2 Everything: the sources

| Kind | Source | What a row holds | Cost |
|---|---|---|---|
| `tool` | srv reads each `tool_use` line at `append_output_line` | tool name plus a one-line summary (below), the agent, the turn, a History link to the exact byte | about 150 bytes a row |
| `turn` (new kind) | the `result` line and the turn start already tracked by the turn tracker | start and end rows with outcome, duration and the cost the Audit already knows (`AuditCost`) | small |
| `thought` | a thinking block | a marker only (duration), no text, linking to the byte in History | small |
| `jekt` | exists | adds an "open this message" link (3.4) | none |
| `action`, `registry`, `agent_*` | exist | unchanged | none |
| sounds | not a kind: see 3.3 | a badge on the row that makes one | none stored |

**Tool summaries** are made at the moment of writing and never copy a raw call: `Bash` shows the first 80 characters of the command, `Read` / `Edit` / `Write` the path, MuxAPI tools their tool and action (`Message send to agenty`), `Vault` the action only. The existing redaction (`wconfig/redact.rs` and the jekt sanitizer) runs over every summary; a summary that still looks like a secret is cut to its tool name.

**Storage and retention.** Tool, turn and thought rows go to a new `<shared>/audit/activity-YYYY-MM-DD.jsonl`, one file a day, so they can be dropped by date. At 2,000 tool calls a day for a busy agent that is about 300 KB per agent per day; 20 agents are 6 MB a day. Recommendation: keep 30 days, then delete the day's file (the transcript still holds the full call, so nothing is lost, only the index). The jekt file keeps forever as it does today. `audit.query` merges the day files the same way it merges the jekt file and the ledger.

### 3.3 Which agent made that sound

Every sound is caused by something already in the log, so a sound needs no row of its own:

| Sound | Row that makes it |
|---|---|
| tool tone | the `tool` row |
| `agent.turn.complete` / `.error` / `.interrupted` | the `turn` end row |
| `agent.message.accepted` / `.rejected` | the `jekt` row (delivered / refused) |
| waiting-for-input loop | the `turn` end row where the agent waits for you; the row stays marked while the loop plays |
| shutdown-pending | no agent: an AgentMux row, "app is closing" |

Three pieces:

1. **A table, one place.** `sound-events.ts` gets `soundFor(kind, outcome)`, the same mapping `sound-service.ts` uses, so the Audit shows a speaker badge on exactly the rows that play, and respects the user's mute settings (a muted event shows no badge).
2. **A ring of what played.** `SoundPlayer.play` and `ToolTonesPlayer.play` each call `noteSound({ blockId, agentId, event, at })`, a 50-entry in-memory ring per window. When a sound plays, the Audit (if open) lights the newest matching live row (same agent, same kind, within 2 seconds) for 3 seconds.
3. **"Last sound".** A status bar item, "♪ Agent3 · Bash · 4 s ago", shown while sounds are on and one played in the last 60 seconds; the same text sits in the Audit header. Click: Audit opens scrolled to the row, with a button for the agent's pane. This answers "who was that" without opening anything first.

Matching by agent, kind and time is approximate when one agent makes two identical calls within the window; the newest wins. Exact matching would need the sound path to carry a row id, and the frontend plays tones from its own stream events, not from Audit rows, so the cost is not worth it.

### 3.4 Links everywhere

Every row's agent name is a link: click opens the live pane (or the History reader if there is no pane), shift-click always opens History. A row's `links` grow to:

| Link | Target | From |
|---|---|---|
| `pane` | the agent's open pane | exists |
| `history` | the exact turn or byte | exists (`at_byte` filled for the new kinds) |
| `message` | the message in History | a `jekt` row opens the receiving agent's History at the time of the jekt (`at_ms`); at the exact byte if the transcript line carries the jekt's `request_id`, which PR 3 confirms before relying on it |
| `task` | `agentmux://task/N` | the task the agent held at that time (the Kanban knows the holder and the claim times); needs #14995's opener |
| `pr` | the PR URL | the held task's `links.pr`, else a PR URL named in the summary (`gh pr create`, `gh pr merge`) |

A link the app cannot resolve is not drawn. Message text is **not copied** into the Audit: the row opens it from History (3.5).

### 3.5 Privacy

- **Kept in the index:** agent, kind, the scrubbed summary above, times, outcome, cost, byte positions. **Never:** a Vault value, a secret, thought text, a tool result.
- **Message text:** the Audit keeps a hash and a length, as today. "Open message" reads the text from the user's own transcript store, so it needs no copy and works only on the machine that holds it.
- **Cloud:** none of this leaves the computer. The MuxBus relay gets no Audit rows.

### 3.6 Tiers: where a row came from, and how it travelled

The operator's words (2026-11-03, via AgentY): "for the audit work, we also need to stay tier-aware .. channel, LAN, and cloud .. do we have a solid method for reconciling and filtering those events?" Not yet. What is true today (read on `origin/main`, 2026-11-03):

| Fact | Where |
|---|---|
| `AuditLogEntry` has no tier, trust, host or version. The delivery has them in hand (`InjectionRequest.delivery_tier`: host / channel / lan / wan; the marker's TRUST) but the entry keeps only the WAN verdict (`wan`) and a LAN-claimed UID. | `reactive/types.rs` |
| `channel` is added to a line only as it is written; `audit.query` ignores it, so every channel that shares the file is merged into one list with no label and no filter. | `audit_store::write_line`, `backend/audit_query/` |
| A delivery row's `request_id` is the MSGID in the message's marker (the handler passes the same id to the log and to `jekt_origin`). A message has one audit row, written where it is delivered. The sender's side is only an `action` row for the MuxAPI call, under a different id. | `reactive/handler.rs` |
| The jekt file is in the account-wide shared folder and is written by every same-version srv; one write call per line, so lines do not interleave. | `audit_store.rs` |
| **#6320 drops a different-version channel's rows.** A srv that gave the shared folder up to an earlier srv of another version gets `resolve_global_shared_root() == None`, so `audit_store::default_path()` is `None`: at start it logs "jekt audit: no shared directory; not kept in a file" and keeps its jekt, action and registry rows only in the in-memory ring (lost at restart). Its `audit.query` gets `jekt_file: None` and no shared transcript store, so its Audit view lists nothing from before. | `bootstrap/background.rs:205`, `registry/paths.rs`, `app_api/audit_query.rs` |

So the answer to "do we have a method": no. The design:

1. **Every row says where it is from and how it came.** `AuditLogEntry` and `AuditEntry` get optional fields (old lines read as before): `host`, `channel`, `version` (the writer's), `tier` (`host`, `channel`, `lan`, `cloud`; today's `wan`, named cloud on screen), `trust` (the marker's TRUST value) and `msg_id` (for a jekt the same as `request_id`, named so a reader need not know). Old lines show "tier unknown", except where the line itself proves it (`wan` present is cloud, `lan_claimed_uid` present is LAN), marked "inferred".
2. **A message is two halves, joined by MSGID.** The sending srv writes a `sent` row at hand-off (tier as sent, `msg_id` the id it will be delivered under); the receiving srv writes the row it writes today (`delivered`, `held`, `refused`, `ack_refused`, ...). The view joins rows with one `msg_id` into one row with two ends: "sent 12:01:03, delivered 12:01:04 (host-verified, channel)". A half whose other end is not visible (the peer is off, or not paired) stays one row marked "other end not seen". The join key is `msg_id`; the duplicate key, for the same line read twice (two channels read the same file, or a line merged with a peer's answer), is `(host, channel, ts_ms, msg_id, kind, outcome)`.
3. **Fix for #6320: a deferred srv keeps its rows.** When the shared folder is not used, the srv appends to its own file, `<its data home>/audit/jekt-audit.jsonl`, and `audit.query` reads the shared file and its own, merged by time and de-duplicated by the key above. A line goes to exactly one file, so the two never repeat each other. Whether a deferred srv's agent start and end rows (from the shared transcript store) can be kept the same way is checked in that PR; the jekt, action and registry rows are the part known to be lost. This is a small PR of its own, first.
4. **Filters and sections.** A scope control at the top of the Audit: **This channel** (the default), **This machine** (every channel here), **LAN** (paired machines too), and a **Cloud** chip that filters by tier, not by place (a message that travelled through the cloud is shown from either end that is visible). Past This channel the list has sticky section labels per host and channel, "host · channel · version", like the Swarm's sections. Counts and totals (message stats) are per section; a peer's rows are never added into this machine's totals.
5. **Other channels and peers are asked, not read from a shared file.** Another channel on this machine is asked over loopback with its `local_url` and key from the host-global registry, exactly as the Swarm reads other channels (`swarm_remote::channel_statuses`), so a channel that stays out of the shared folder is still reachable. A LAN peer is asked on demand with a typed read-only `audit.query` over the signed remote-op route (`/agentmux/remote/op`, where the signature authorizes the request, never the key alone), answered only when the peer's owner turned on `audit:sharewithpaired` (default off, like `tower:sharewithpaired`). The answer is the same rows, scrubbed the same way (summaries, never message text). A peer's rows are fetched, shown and forgotten here; they are never relayed through the cloud, and the cloud carries no audit rows at all. A cloud message is logged locally at both ends, with tier cloud, which needs nothing more. The live tail covers this channel by push; other channels and peers refresh every 5 to 10 seconds while their scope is shown.

## 4. Build: five PRs

0. **A deferred srv keeps its rows (3.6.3).** Own audit file when the shared folder is not used; `audit.query` reads both. A failing-first test: a deferred srv appends and queries. Small, and it fixes a loss that exists today.
1. **Index and provenance (srv, Rust).** `activity` writer at `append_output_line` (tool, turn, thought rows; scrubber; day files; 30-day delete); the new provenance fields and the `sent` row (3.6.1, 3.6.2); `audit.query` answers `tool` / `thought` / `turn` and joins by `msg_id`; tests with recorded stream-json lines, including a secret in a command. Rust: built and tested on charlie (narko is off Rust builds).
2. **Live and scope (srv event + view).** `AuditEntry` broker event (batched), the live tail, pause, "N new", reconnect catch-up; the scope control and the host / channel sections for This channel and This machine (loopback) (3.6.4).
3. **Sounds and links.** `soundFor`, `noteSound` ring, row highlight, "last sound" in the status bar and header, the agent / message / task / PR links.
4. **LAN peers (3.6.5).** The typed `audit.query` remote op, `audit:sharewithpaired`, the LAN scope with the peer's sections.

Each PR names the section it covers and runs the checks it touches.

## 5. Open questions for the operator (recommendation first)

1. **Retention of tool rows.** Recommend 30 days (index only; the transcript keeps the call). Alternative: forever, about 2 GB a year at 20 busy agents.
2. **Thoughts.** Recommend a marker with no text. Alternative: the first 80 characters, which can leak a plan or a secret the agent was reasoning about.
3. **Message previews.** Recommend none (open from History). Alternative: a setting `audit:keepmessagepreview` (default off) that stores the first 140 characters.
4. **"Last sound" location.** Recommend the status bar plus the Audit header. Alternative: the Audit only.
5. **Mute from the answer.** A "Mute this agent's sounds" action on the last-sound item would fix the annoyance at its root, not only explain it. Recommend adding it in PR 3 (a per-agent entry beside the `notify:*` settings).
6. **Agents on other computers.** Now covered by 3.6.5 (PR 4): a peer's rows come over the LAN on demand. Their tool calls are written on their own machine and arrive the same way.
7. **Container agents.** Recommend the same path: their output also goes through srv. To be confirmed in PR 1 before the writer is trusted for them.

8. **Who may ask for my audit rows over the LAN.** Recommend the signed remote-op route, read-only, off until `audit:sharewithpaired` is on. Alternative: the LAN key, as `agent-names` does. That is too wide for a who-did-what record: any machine holding the key could read it.
9. **Old lines with no tier.** Recommend "tier unknown", or an "inferred" tier where the line itself proves it (cloud from `wan`, LAN from `lan_claimed_uid`). Alternative: call every old line local, which is wrong for the ones that were not.
10. **Default scope.** Recommend This channel (what you are looking at), with This machine one click away. Alternative: This machine by default, which shows every channel's rows mixed in, as the Audit does today.
11. **A `sent` row per jekt.** It adds about one row, around 200 bytes, to every message and gives the Audit its two ends. Recommend yes. Alternative: join an `action` row to the delivery row by time, which is a guess.

## 6. Not here

Searching inside tool inputs and results (the History search does that), exporting the log, per-row annotation, and changing what the jekt file records.
