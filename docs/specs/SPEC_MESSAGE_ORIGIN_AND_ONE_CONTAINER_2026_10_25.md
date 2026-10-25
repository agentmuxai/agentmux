# SPEC: Only the human's own words look like the human: one message origin, set by srv, and one container for everything else

**Date:** 2026-10-25
**Author:** Agent1 (UI kit, theme), for Kanban #4472; the build is #4473.
**Status:** draft, for the operator. Nothing is built. Written against `origin/main` @ `1db5871a0`; file:line citations were read on that commit, spot-check them before relying on one.
**Trigger:** The operator, 2026-10-25, with a screenshot of an agent pane on 0.60.1: a jekt (agent1 to agentx) and an "[AgentMux] AgentMux was upgraded…" note both render in the inverted "user turn" style. *"Only Human text looks like this: everything else should be encapsulated in a jekt-like container. This should be a standard across the entire framework. Only humans get the inverted color. Let's do an audit across how all messages (are handled), we need clear lines that are both DRY and set up styles efficiently."*
**Related:** `SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` (the jekt bubble, §3), `SPEC_SWARM_BROADCAST_AS_USER_MESSAGE_2026_10_01.md`, `SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md` (§5: `TurnOrigin`, `TriggerKind`), `SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md`, `SPEC_AGENT_INPUT_HISTORY_2026_10_18.md`. This is the same boundary as the rule that nothing may pass itself off as the operator (the jekt trust model, `crates/srv/src/backend/reactive/sanitize.rs`).

## 0. Summary

- **What the screenshot shows, and why.** The jekt is not rendered by the jekt bubble because the pane decides "is this a jekt" with a regex that must match the **whole** message: `JEKT_BLOCK_RE` ends in `\[\/JEKT\]\s*$` (`frontend/app/view/agent/stream-parser.ts:63`). srv appends its held `[AgentMux]` notes **after** the message it delivers (`held_notes::with_notes`, `crates/srv/src/backend/reactive/held_notes.rs:137`, called from `deliver_agent_message_from`, `crates/srv/src/backend/blockcontroller/mod.rs:787`). A jekt with a note after `[/JEKT]` no longer matches, so the whole turn falls through to the plain `user_message` node, which is the inverted style. A note on its own has no node type at all, so it is a `user_message` too.
- **The cause is not that one regex.** srv already knows what every input is (`TurnOrigin`, `TriggerKind`, the `InjectionRequest` fields), but it does not write that into the transcript. The record holds the text, and every viewer re-derives what it is by parsing the text: the pane (three regexes), history (`typed_by_a_person`, a 14-prefix list), the turn ledger (`TurnTrigger::from_input` reads text), the mobile viewer (nothing). Anything that parses text can be fooled by text, and a viewer that parses less than another shows something else.
- **The design.**
  1. srv writes a **typed origin** on every user record it writes (`amx.origin`), built from the typed values it already holds when it composes the message, and the srv-written notes as their own list (`amx.notes`), **signed by srv** so a line an agent appends to the file by hand is not believed (§6.1). Never inferred from the body.
  2. The pane reads it. Only `origin.kind = user` (and, if the operator agrees, `broadcast`) gets the human style. A record with no origin, or one this build can't read, renders in the **container**, never as the human.
  3. **One container component** with a header row (origin, sender, trust or signature badge, time, late delivery) and variants by origin. One set of theme tokens. The human style gets its own tokens, referenced from one stylesheet, and a test fails if anything else uses them.
- **Phases.** P0 is a small interim fix for the screenshot (a day). P1 srv stamps the origin. P2 the pane and the container. P3 the providers whose CLI echoes the prompt (Gemini, ACP, Codex App Server). P4 other viewers (mobile, history, Swarm) and the text classifiers deleted. §9.
- **Decisions for the operator:** nine, §11, each with a recommendation. The ones that change what you see: how a Swarm broadcast looks (D1), how old transcripts look (D2), what Gemini, ACP and the Codex App Server do until P3 (D8), and the signing key's restart cost (D9).

## 1. What the screenshot shows

A Claude agent pane. Above: a tool row. Then one block, inverted colours, an orange left border (the user-turn style), holding:

1. the raw jekt: `[JEKT:FROM=agent1 TO=agentx TIER=coord DELIVERY=host TRUST=host-verified MSGID=… PRIORITY=normal TS=…]`, the divider lines, `From: … | To: … | ts=…`, the body, `Reply: …`, `[/JEKT]`;
2. directly after it, a blank line and `[AgentMux] AgentMux was upgraded from 0.60.0 to 0.60.1 at 14:17. …`.

That is one user record: srv delivered the jekt and attached the held upgrade note to it (`held_notes::attach`, `blockcontroller/mod.rs:799`). Two defects follow from one cause:

- the jekt is shown as the human because `JEKT_BLOCK_RE` is anchored to the end of the message (§2.2);
- the note is shown as the human because nothing recognises `[AgentMux]` on the render side at all.

## 2. Audit

### 2.1 Every way text enters an agent's conversation (srv)

"Origin recorded" means a typed value that exists in srv at the point of production. "In the record" means it reaches the transcript line the viewers read. Today the answer to the second is **no for every row except the device row**: the record is `{"type":"user","message":{"role":"user","content":"…"}}`, plus `amx` for two cases (below).

| # | Input | Produced by | What marks it in the text | Typed value in srv | In the record |
|---|---|---|---|---|---|
| 1 | The human typing in the pane | the pane's send path to `send_user_message_outcome_from` (`persistent/input.rs:142`); recorded by `persist_user_line` (`transcript/append.rs:62`) | nothing | `TurnInput.origin = User` (`blockcontroller/health.rs:26,42`); input history `source = composer` | `echo: "stdin"` on the published event; nothing in the file (to verify, §12) |
| 2 | Swarm broadcast typed by the human | `broadcast_turn_message` (`reactive/sanitize.rs:541`); LAN: `swarm_lan_broadcast.rs:84` | `[BROADCAST:FROM=user VIA=swarm TO= RECIPIENTS= MSGID= TS=]` first line | `TurnOrigin::User` plus the marker text decides `TriggerKind::Broadcast` (`health.rs:128`) | no |
| 3 | Jekt from an agent: host, channel, LAN, WAN | `inject_message_inner` (`reactive/handler.rs:799`) composes the text in `wrap_jekt_message_with_files` (`reactive/sanitize.rs:408`) | `[JEKT:FROM= TO= TIER= DELIVERY= TRUST= MSGID= PRIORITY= TS= HELD_FOR= INSTANCE= INSTANCE_STATUS= SIG= ESCALATE= FILES=]`, divider, `From:` header, body, `Reply:` footer, `[/JEKT]` | `InjectionRequest` (`reactive/types.rs:37`): `source_agent`, `delivery_tier`, `sig_verified`, `reagent_verified`, `lan_verified`, `channel_verified`, `wan_verified`, `audit_source_uid`, `held_sent_at_ms` | no |
| 4 | Review and CI notices | github-consumer, arrives as a WAN jekt with `SIG=verified` | same as 3, `FROM=github-consumer` | as 3; `TriggerKind::Service` is chosen by `FROM` ending `-consumer` (`health.rs:134`) | no |
| 5 | `[AgentMux] ` system notes (upgrade, cloud paused/resumed, context size, expired messages) | `muxbus/delivery_status/notes.rs` (`NOTE_MARKER`), held per pane and attached to the next message by `held_notes::attach` (`held_notes.rs:270`) | `[AgentMux] ` at the start of a line, after the message and `\n\n` | the `HeldNotes` entry (key, text, `NoteKind`) | no |
| 6 | Late-delivery prefix `(sent 14:30, delivered 14:41) ` | `notes::delayed_prefix` (`muxbus/delivery_status/notes.rs:209`) | the prefix, in front of a note or jekt | the two timestamps | no |
| 7 | Worker notes ("Agent4 turn ended", exit, failed, awaiting user) | `worker_notes.rs:65` | `[AgentMux] ` | `WorkerEvent` | no |
| 8 | Overlap notes ("Agent4 also has uncommitted changes in …") | `work_facts/overlap_notes.rs:137` | `[AgentMux] ` | `NewEdit`, rate limiter | no |
| 9 | Kanban / task notices ("Task #N can start", leader nudges, "the operator is following") | task and leader-nudge code, delivered as jekts or notes | jekt marker or `[AgentMux]` | the sender | no |
| 10 | Schedule and Jobs | `cron/mod.rs` `fire_message` (`:136`), `deliver` (`:98`) | jekt marker `FROM=cron` or `FROM=loop`; `[run:…]` brief | `source_agent = "cron"`, `request_id`, `audit_source_uid` | no |
| 11 | Supervisor continuation | `supervisor_turn_message` (`reactive/sanitize.rs:567`) | `[SUPERVISOR:MODE= N= OF=]` first line | `TriggerKind::Supervisor` (text prefix test, `health.rs:164`) | no |
| 12 | Held-message replay after a quota reset, sign-in or idle hold | `jekt_held.rs` `replay_pass` (`:164`) | original jekt marker with `HELD_FOR=Ns`; `TS` is the original send time | `held_sent_at_ms` | no |
| 13 | Resume / catch-up after compaction, fresh start, upgrade; memory reinjection | the frontend composes `composeReinjectionMessage` (hidden), srv `continuity.rs`, `continuity_state.rs` | a `<system-reminder>` wrapper | `TurnOrigin::System`, `TriggerKind::System` | no (the pane hides it by matching the wrapper, `memory-reinjection.ts`) |
| 14 | Widget `agents:send` | `widget_access_handlers.rs:150` | jekt marker, `FROM=widget:<id>`, `TRUST=self-declared` | `source_agent = "widget:<id>"`, no signature | no |
| 15 | Attachments and files on any of these | `<attached_images>` / `<attached_files>` list appended to the text (`attachments/prompt.rs`) | the list text | the attachment ids | no (the pane splits the list off by text, `splitAttachedImages`) |
| 16 | A leader relaying the operator's quoted words (Kanban `accept` / `send_back` with `user_instruction`) | an ordinary jekt from the leader | jekt marker with the leader as `FROM`; the quoted words are body text | none beyond "a jekt from the leader" | no |
| 17 | Hook output and the CLI's own injected text (`<task-notification>`, `<user-prompt-submit-hook>`, `<local-command-*>`) | the CLI, not srv | angle-bracket wrappers | none (srv only sees them in the CLI's stream) | no |
| 18 | Message from a paired device (the mobile viewer) | `viewer/origin.rs:85` `stamp`; delivered by `http_viewer_input.rs:73` as `TurnOrigin::User` | none | `device`, `message_id` | **yes**: `amx: {via: "viewer", device, message_id}`. This is the human, on their phone: it maps to `origin.kind = user` with a "from <device>" label (as `UserMessageBlock` shows today), never untyped |
| 19 | Any "message no pane typed" on the CLIs srv writes the record for (Claude, Codex, Kimi, Antigravity, Qwen) | `UserRecordSink::write` (`blockcontroller/subprocess/mod.rs:505`), `injected_user_record` (`:477`) | none | `shown_by_pane` boolean | **yes, one bit**: `amx: {via: "inject"}`; the pane then renders the record live (`isInjected`, `providers/translator.ts:56`) |

Two facts make the build small:

- **Every injected delivery passes through one function**, `deliver_agent_message_from(block_id, message, origin: Option<TurnInput>, inline)` (`blockcontroller/mod.rs:787`), which is also where notes are attached. It is the place to attach a typed origin to what the controller records.
- **Two record writers cover the CLIs srv records for:** `persist_user_line_as` (`transcript/append.rs`, where `viewer::origin::stamp` already adds `amx`) and `UserRecordSink`. Both already know whether the pane typed the message.

### 2.2 How the pane decides today

Rendering is already keyed on the **node type**, not on text: `DocumentNodeBody` switches on `node.type` (`frontend/app/view/agent/components/DocumentRow.tsx:220-407`): `user_message` → `UserMessageBlock`, `jekt_message` → `JektBubble`, `agent_message` → `AgentMessageBlock`, `memory_reinjection`, `context_delivery`, `ambient_narration`. The defect is how the **node type is chosen**, in the stream parser (`stream-parser.ts` `userMessageToNode`, `:860`):

1. `tryParseMemoryReinjection`: matches a hidden `<system-reminder>` wrapper.
2. `tryParseJekt` (`:917`): `JEKT_BLOCK_RE` over the whole trimmed message (`:63`). Needs `FROM` and `TO`. Direction (incoming/outgoing) is **inferred** by comparing `FROM` with the pane's agent (`:940`). `TIER`, `DELIVERY`, `TRUST` come from the marker text, defaulting to the least trusted value.
3. `parseBroadcastHeader` (`:80`, `BROADCAST_HEADER_RE` `:73`): only the exact shape `FROM=user VIA=swarm` counts; this is the one place a lookalike is deliberately refused.
4. `parseSupervisorHeader` (`view/agent/supervisor.ts:83`).
5. `STARTUP_HEADING_RE` (`:52`): `# Session Context` makes a `user_message` the collapsed startup variant.
6. Otherwise: `user_message`, the inverted style (`_document-nodes.scss:1308-1528`: `background: var(--main-text-color); color: var(--block-bg-solid-color); border-left: 3px solid var(--user-input-color)`).

`[AgentMux]` is not in this list. So every system note, and every jekt with a note after it, ends at step 6.

### 2.3 Where the style is chosen by parsing text, not by origin

Every item is a place a message can change what it looks like or is counted as by what it says:

| Where | What it parses |
|---|---|
| `stream-parser.ts:63` `JEKT_BLOCK_RE` | the jekt block, the whole message |
| `stream-parser.ts:73` `BROADCAST_HEADER_RE` | the broadcast header |
| `view/agent/supervisor.ts:83` | the Supervisor header |
| `stream-parser.ts:52` `STARTUP_HEADING_RE` | `# Session Context` |
| `stream-parser.ts:143` `stripJektEnvelope` | the divider, `From:`, warning and `Reply:` lines |
| `stream-parser.ts:940` | direction from `FROM` vs this agent |
| `view/agent/memory-reinjection.ts` | the hidden `<system-reminder>` wrapper |
| `view/agent/attachments/attached-images.ts` `splitAttachedImages` | `<attached_images>` / `<attached_files>` |
| `crates/srv/src/backend/blockcontroller/health.rs:128-190` `TurnTrigger::from_input`, `is_supervisor_input`, `jekt_sender` | the broadcast marker, the Supervisor prefix, `FROM=` (for the working row's "↳ jekt from X" and the notification router) |
| `crates/srv/src/backend/history/typed.rs:17-52` `typed_by_a_person` | 14 prefixes (`[JEKT:`, `[AgentMux]`, `[BROADCAST:`, `<system-reminder>`, …) and the `(sent …)` prefix, to decide what the person typed (input history) |
| `crates/srv/src/backend/reactive/held_notes.rs:148` `strip_attached` | the notes srv appended, by a hash log |
| `crates/mcp` and `history::adapter` (`Agents search_history`) | return the text; no kind |

### 2.4 The other viewers

- **Persisted record.** `transcript/append.rs`: one NDJSON line per message, no origin field except `amx` (rows 18 and 19). An old record, or one from a CLI that echoes its prompt, has none.
- **History and search.** `HistoryMessage { role, content, timestamp, tool_uses }` (`history/adapter.rs`, `record.rs`): no origin. Input history rebuilds "what a person typed" by text (§2.3).
- **Mobile viewer** (`agentmux-mobile`): receives raw transcript lines (`lib/core/viewer/feed_events.dart`) and has no jekt rendering. It shows what the text says. (To verify, §12.)
- **Swarm, Kanban, My Agents:** they show agent state and one summary line, not message bodies; the only message-origin text is the turn trigger lead-in ("↳ jekt from X") from `turn-trigger-text.ts`, driven by the typed `TurnTrigger` (this is the good example: srv publishes `kind`, `from`, `external`, and the pane never parses it).
- **Provider coverage.** `persists_user_record` is true for `claude-stream-json`, `codex-json`, `kimi-stream-json`, `agy-stream-json`, `qwen-stream-json` (`subprocess/mod.rs:458`). Gemini's CLI echoes the prompt itself, and ACP and the Codex App Server run through other controllers; `deliver_agent_message_from` says ACP and App Server turns "carry no provenance on any path today, the user's own typed turn included" (`blockcontroller/mod.rs:787`). So for those providers srv does not write the user record and there is nowhere to put the origin yet (P3).

## 3. One origin model

### 3.1 The type

srv already has two enums built for this and not stored:

- `TurnOrigin { User, Automated, System }`: the **security class** (who delivered it).
- `TriggerKind { User, Broadcast, Agent, Service, Schedule, Task, Supervisor, System }` with `from`, and `external` (`health.rs:75-160`): the **display kind**, which `turn-trigger-text.ts` already words.

Reuse `TriggerKind` as the message's origin kind and extend it where §2.1 has no home. A new typed `MessageOrigin`, exported to TypeScript like `TurnTrigger`:

```
MessageOrigin {
  kind: user | broadcast | agent | service | schedule | supervisor | task | system | widget,
  from?:        string,     // display name: the agent, "cron", "github-consumer", "widget:<id>"
  trust?:       jekt trust, // host-verified | channel-verified | lan-verified | wan-verified |
                            //   unverified | self-declared | network-claimed
  tier?:        info | coord | sensitive,
  delivery?:    host | channel | lan | wan,
  sig?:         verified | invalid,
  msg_id?:      string,
  sent_ms?:     number,     // when the sender sent it
  delivered_ms?: number,    // when srv delivered it; late if far from sent_ms
  held_for_s?:  number,
}
```

`kind = widget` is new (row 14: today it is an `agent` kind with a `widget:` name); `task` and `supervisor` exist. The values are the ones srv holds when it composes the message (`InjectionRequest`, the `NoteKind`), so no value is read back from the text.

### 3.2 Where it is stored

On the record srv already writes, in the `amx` object that `viewer::origin::stamp` and `injected_user_record` already use:

```json
{"type":"user",
 "message":{"role":"user","content":"[JEKT:FROM=agent1 ...]\n...\n[/JEKT]\n\n[AgentMux] AgentMux was upgraded ..."},
 "amx":{"v":1,"via":"inject","ts_ms":…,"sig":"<HMAC, §6.1>",
        "origin":{"kind":"agent","from":"agent1","trust":"host-verified","tier":"coord","delivery":"host","msg_id":"…","sent_ms":…,"delivered_ms":…},
        "notes":["[AgentMux] AgentMux was upgraded from 0.60.0 to 0.60.1 at 14:17. …"]}}
```

- `message.content` stays exactly what the agent received, so replay, History search and anything that reads it keep working.
- `amx.origin` is the origin of the input itself. A pane-typed message gets `{"kind":"user"}`; a Swarm broadcast `{"kind":"broadcast",…}`.
- `amx.notes` is the list of notes srv attached (`held_notes::attach` already has them as a list before it joins them with `with_notes`). A viewer compares the **exact** suffix `"\n\n" + notes.join("\n")` (byte for byte, no trimming, no fuzzy match). If `content` ends with it, the part before it is the main part and each note is its own container. If it does not (a record cut or edited), the **whole content** is shown in **one** container of the record's origin (variant "Message with notes"), never as the human, even when the origin is `user`: nothing is guessed, and nothing a body does can move text into the human style.
- `amx.v` is the shape version. A record with no valid `amx.origin`, or a `v` or `kind` this build does not know, is **untyped** (§7). A record with `amx.via = "viewer"` and a valid signature (§6.1) is read as `origin.kind = user` with the device label (the paired phone is the human); new records write `origin` as well.
- `via` keeps its two existing values (`viewer`, `inject`); the device fields stay. `origin` is additive, so an older pane ignores it, and an older record without it keeps working through §7.

### 3.3 How it is set (srv, the producers)

At the one function where inputs and notes meet:

- `deliver_agent_message_from` (`blockcontroller/mod.rs:787`) gains a typed `MessageOrigin` in place of (or inside) the existing `Option<TurnInput>`. Each producer in §2.1 passes the value it already has: `inject_message_inner` builds it from the `InjectionRequest` (the same fields that become the marker); the Swarm broadcast and Supervisor paths build `broadcast` and `supervisor`; cron builds `schedule`; the pane's send path builds `user`; the frontend's reinjection builds `system` (as `TurnOrigin::System` does today).
- The controller passes it to the record writer (`persist_user_line_as`, `UserRecordSink`), which writes `amx.origin` and `amx.notes`.
- `TurnTrigger::from_input` (the working row) is then built **from** the `MessageOrigin`, so the two stop being separately derived: the text tests in `health.rs` (`jekt_sender`, `is_supervisor_input`, the broadcast `contains`) go away.

A producer that does not set an origin (a new code path nobody updated) is a compile error: the parameter is not optional. This is what keeps the set complete as code is added.

### 3.4 The human is a kind, and what proves it

`kind = user` and `kind = broadcast` are claims the whole design rests on, so the build proves them rather than assuming them. What the code on `origin/main` does today (read, not run):

| Who may produce a `User`-origin input | Where | Guard today |
|---|---|---|
| The pane's composer: RPC `agentinput` | `server/agent_handlers/input.rs:1421`: `User`, or `System` when the request says `hidden` | **None on the caller**: the handler is registered with an ignored context (`move |cmd, _ctx|`, `:1388`), unlike `fleet.broadcast`, which calls `ctx.operator_only(...)` (`app_api/fleet.rs:94`). The MCP tools do not expose `agentinput` (no reference in `crates/mcp`). Whether anything in front of the RPC engine refuses a non-window caller was not checked. |
| Swarm broadcast to this machine and to another channel | `app_api/fleet.rs:346,430` `deliver_broadcast_turn`, origin from the caller | `ctx.operator_only` on `fleet.broadcast` |
| The operator's Kanban "tell" | `app_api/kanban.rs:373`, `TurnOrigin::User` | to check |
| The paired phone | `http_viewer_input.rs:73`, `TurnOrigin::User` | the viewer's pairing token |
| Supervisor continuation | `supervisor/watcher.rs:431`, `Automated` | srv's own watcher |
| Everything that goes through `inject_message_inner` | `reactive/handler.rs:799` | sets `Automated`; cannot set `User` |

The `send_user_message`, `send_user_message_outcome` (origin `None`) and `send_user_message_with_images_from` family is called from `deliver_agent_message_from` and the persistent controller's own queue (`persistent/input.rs:120-160`, `queue.rs`); `deliver_agent_message_from(block_id, message, None, …)` is the `origin = None` path (`blockcontroller/mod.rs:782`, `bootstrap/background.rs:225`).

**The `agentinput` gate is P0, the rest of this is the start of P1** (AgentY reviews it; it is the security boundary). The route that ignores its caller and defaults to `User` is the live forge path today, so it does not wait for the typed record:

1. List every caller of the `send_user_message*`, `deliver_agent_message*` and `route_agent_message*` family and every constructor of `TurnInput`, and say for each who can reach it.
2. Make the origin **required and never caller-supplied**: `kind = user` is set by srv from the authenticated caller class (the operator's window, a paired device), not from a request field. `hidden` may only lower a `User` input to `System`, as now, never raise.
3. Gate `agentinput` (**moved to P0**, below): refused for an agent caller and any caller that is not the operator's window, or stamped with that agent; **never `user`**. Origin comes from the caller context: the operator-proven window is the human, an agent token is that agent, anything else is unknown (the container).
4. An input with `origin = None` today (`mod.rs:782`, `background.rs:225`) gets an explicit kind, not a default.
5. **Test:** an agent-token (and a non-window) call to the `agentinput` route, to `fleet.broadcast` and to the Kanban tell is refused or recorded with an agent kind; a request that names `kind: user` in any field is ignored. One test per route, written to fail on today's `agentinput`.

Today's quoting already stops a delivered body from writing a marker (`reactive/sanitize.rs:600-660` rewrites `[JEKT:`, `[BROADCAST:`, `[SUPERVISOR:` and `[AgentMux]` inside a body to `…-QUOTED`); the typed origin makes the render independent of that quoting working, so the two protect each other.

## 4. One container style

### 4.1 The rule

- **Human style** (inverted): only `origin.kind = user`, and `broadcast` if D1 says so. It has its own tokens (§4.3) and one stylesheet.
- **Container:** every other message, every untyped record, every unreadable origin.

### 4.2 The container

One component (replacing `JektBubble` and the ad-hoc notes), `OriginContainer`, in `frontend/app/view/agent/components/`:

```
| icon  ORIGIN-LABEL  sender        [trust badge] [sig]  time  [delivered 11m late] |
| body (collapsed by the existing disclosure rules for long bodies)                   |
| attachments (the existing strip)                                                    |
```

- **Header row:** the origin icon and label (from `kind`), the sender (`from`), the trust badge (`trust`, with the `tier` shown the way the bubble does today: a `sensitive` jekt keeps its red edge), a signature badge when `sig` is set, the time, and "delivered 11m late" when `delivered_ms - sent_ms` passes the same threshold srv uses for the `(sent …, delivered …)` prefix (`DELAYED_AFTER_MS`), computed from the two typed timestamps, not by reading the prefix.
- **Variants by origin** are a `data-origin` attribute and a class on the header, not per call site:

| Origin | Label | Notes |
|---|---|---|
| `agent` | "Jekt from X" | today's jekt bubble: tier edge, trust badge, direction (outgoing is dimmed) |
| `service` | "Notice from github-consumer" | the signature badge matters here (`SIG=verified`) |
| `schedule` | "Scheduled: cron" / the job name | |
| `supervisor` | "Supervisor" | shows mode and n of m, as the human-style note does today |
| `task` | "Task notice" | |
| `system` | "AgentMux" | notes, reinjection, a hidden side question |
| `widget` | "Widget X" | `self-declared`, so always the unverified badge |
| untyped | "CLI message" or "Earlier message" | §7: quiet neutral edge, no warning colour |

- **Notes after a message** (`amx.notes`) render as `system` containers directly under the message part, compact (one line each, no collapse unless long), so a human message with an upgrade note under it shows the human's words in the human style and the note in a container, not one blob.
- **Disclosure:** the existing rules stay (`disclosure.ts`: a jekt is collapsed by default and a `sensitive` one open, a live-arrived one stays open until scrolled off). They apply to the container's body, whatever the origin.
- **The relayed operator's words** (row 16): a jekt from the leader, so an `agent` container. The quoted words are the body; nothing in the container, header or colour says "operator". If a leader's tool marks the quote, it is a labelled quote block inside the body (a plain blockquote with the label "quoted by <leader>"), never the human style.
- **Pending (queued) messages** (`PendingMessagesPanel`, `_pending-footer.scss`): these are the human's own words waiting; they keep the human's tokens at a reduced strength and the warning edge, and move to the same stylesheet.

### 4.3 Tokens

- **Human:** `--human-turn-bg`, `--human-turn-fg`, `--human-turn-edge`, defined once in `theme.scss` (today the inverted style borrows `--main-text-color`, `--block-bg-solid-color` and `--user-input-color` directly). A test (`human-turn-tokens.test.ts`, §10) fails if any stylesheet other than the human-turn one references them. That makes "only humans get the inverted colour" a property the build checks, not a convention.
- **Container:** `--origin-bg`, `--origin-edge`, `--origin-fg`, `--origin-label-fg`, plus one `--origin-edge-<kind>` per variant that differs from the neutral edge, each defined in `theme.scss` as an alias to an existing theme token (`--accent-color`, `--accent-bg-subtle`, `--secondary-text-color`, `--warning-color`, `--error-text-color`, …) so all 13 themes follow without a per-theme override, and the contrast tests in `themes/` cover them. No raw colours (the UI-primitives ratchet).
- **Motion:** none of its own; the live-arrival highlight uses the existing `--duration-*` tokens and the app's motion tiers (`motion.ts`), not `prefers-reduced-motion`.

## 5. DRY: what is deleted or merged

| Now | After |
|---|---|
| `JektBubble.tsx` and its styles `.agent-jekt-bubble` (`_document-nodes.scss:1036-1133`), `CollapsibleMessage` jekt prefix | `OriginContainer` (the jekt bubble becomes its `agent` variant) |
| `UserMessageBlock`'s notes: broadcast, supervisor, fromDevice (`UserMessageBlock.tsx:208-230`) | the human turn keeps a one-line label only for `broadcast` and `fromDevice` (D1); the Supervisor note becomes the `supervisor` container |
| The `memory_reinjection` card and its hidden-wrapper test | a `system` container, by origin; the wrapper test goes |
| `context_delivery` and `ambient_narration` cards | **to check in P2**: if they are system-origin text, they become `system` variants |
| `.agent-pending-item` styles (`_pending-footer.scss:17-62`) | the human-pending variant in the human-turn stylesheet |
| `JEKT_BLOCK_RE`, `BROADCAST_HEADER_RE`, `parseSupervisorHeader`, `stripJektEnvelope`, direction-from-`FROM` (stream-parser, `supervisor.ts`) | deleted after the legacy window (P5); until then used only for untyped records (§7) |
| `TurnTrigger::from_input`'s text tests, `jekt_sender`, `is_supervisor_input` (`health.rs`) | the trigger is built from the `MessageOrigin` |
| `typed_by_a_person` and its 14-prefix list (`history/typed.rs`; Agent5's) | "typed by a person" is `origin.kind = user`; records with no origin keep the old test (§7) |
| `held_notes::strip_attached` hash log | not needed for new records (`amx.notes` holds them); kept for old input history |
| `splitAttachedImages` text split | the attachment ids are already typed on the send side; the split stays for old records |

Tokens: the three human tokens and the container tokens replace the direct use of `--main-text-color` / `--block-bg-solid-color` / `--user-input-color` / `--accent-bg-subtle` / `--warning-bg-subtle` in message styles.

## 6. Security

This is the operator's rule that nothing may pass itself off as the operator, applied to the render.

- **What forging is today:** a body can't write a marker (srv rewrites it to `…-QUOTED`), but a body that srv failed to quote, a record from before the rewrite existed, a CLI's own injected text, or a note that isn't recognised, all fall to the human style. That is the fail-open default this spec turns around: **unknown means container.**
- **After:** the style comes from `amx.origin`, written by srv, so no text in a body, a jekt, a note, a hook or a pasted message changes the origin. A message that says it is from the operator, or starts `[BROADCAST:`, `[AgentMux]`, `[JEKT:` or `[SUPERVISOR:`, in any case, spacing or full-width form, is rendered in its container with the origin srv recorded.
- **The record's `amx` versus the body:** a body can contain the characters `"amx":{"origin":{"kind":"user"}}`, but the record's own `amx` is a field of the record, not of the content string, so it cannot be set from the body. A test pins this (§10, S5). That says nothing about someone writing the record's file; §6.1 does.
- **Another instance's stored data:** a LAN or WAN peer sends a jekt, not a record; the receiving srv writes the record and the origin. A peer cannot choose `kind`.
- **Operator approval flows** (Kanban `accept` / `send_back`, the self-quit and App upgrade gates) already decide on the typed origin of the turn, not on a quoted phrase; this does not change them.

### 6.1 A record written by hand (found in AgentY's security review)

"Only srv writes `amx`" holds for the protocol, not on disk. The transcript lives under the user's home and every agent runs as that user, so an agent that can write a file can append a `{"type":"user",…,"amx":{"origin":{"kind":"user"}}}` line, and a pane that trusts `amx` would show it as the human. (Today it is worse: any appended `{"type":"user"}` line replays as the human with no `amx` at all; this design makes that line a container, a strict improvement before it is any stronger.)

The fix is in the design, before P1:

- **srv signs the origin.** For every record it writes, srv adds `amx.sig = HMAC-SHA256(key, "v1" | zone_id | ts_ms | sha256(content) | canonical(origin) | canonical(notes) | via | device)` and `amx.ts_ms`.
- **The key, stated honestly.** Every agent runs as the same OS user, so **a key kept in a file anywhere under the user's home is readable by an agent**, and the F1 work protects what lives in memory and the environment, not a file. If the key were in the jekt-signing key family's file, an agent could read it and forge a valid signature claiming `user`: the one thing the operator forbids. So the key is **not** a plain file. Two options (D9):
  - **(a) A per-run key in srv's memory only.** Nothing on disk to read. Cost: after an srv restart, records written in earlier runs do not verify, so they render as "Earlier message" in the container, **including the human's own messages from before the restart**, until the history rolls over (consistent with D2, but visible daily).
  - **(b) An OS store bound to the AgentMux binary where one exists, with (a) as the fallback where none does.** macOS Keychain with an access control list by code signature is app-bound; Windows DPAPI and Linux libsecret are bound to the user, not the app, so an agent as the same user can read them, and those platforms use (a). Cost: on Windows and Linux (the operator's main platform today) the restart cost of (a) applies; only macOS keeps the human style across restarts.
  - Neither is a MAC you can keep forever and also keep out of an agent's reach on Windows and Linux. That is a property of running agents as the user, not of this design; the spec does not claim more.
- **srv verifies on every read path, and clients never see the raw file.** Wherever srv serves a record to a viewer (the pane's replay and live feed, the mobile feed, `Agents search_history`), it first **removes the record's whole `amx` object** and puts back its own `{origin, notes, device, verified: true}` only if the MAC verifies. A record with a missing, wrong or copied-onto-other-content MAC is served with no `amx`: untyped, so the container. A hand-appended `"amx":{"verified":true}` is stripped by the same rule, because the whole object is removed before anything is added.
- **What a valid MAC does not stop:** an agent copying a real, signed human line from the same zone's file into the same file again would re-show a message the human really typed; it cannot make new human text. Binding the zone and `ts_ms` into the MAC makes a copy to another agent's zone fail.
- **The MAC binds the zone the line is written to, not the pane's block id** (Agent5): the global transcript zone is keyed by the agent (`agent_current_zone(definition id)`), so a human line written from one pane must still verify when the same agent is reopened in another pane. P1 confirms which id `persist_user_line_as` has in hand at write time. **Archive, clear and restore** into the same zone keep line bytes and verify. **A fork** copies the parent's zone into the fork's zone byte for byte (`fork_history.rs`, `copy_files_to_zone`); the copied lines were signed for the parent's zone, so they would not verify in the fork's. Resolution (a P1 detail, not an operator decision): at copy time srv **verifies each `type:user` line against the parent's zone and re-signs only those that verify** for the fork's zone, never an unverified one, so a forged line cannot be laundered by forking; the fork then shows the human's own earlier messages as the human. **Transcript backfill** seeds a zone from provider session bytes, which carry no `amx`: untyped by design, the quiet container. No path shortens or compacts an `output` zone in place (Agent5's read of `origin/main` `d34ce7431`: session archive and restore, backfill and fork are the only whole-file operations).
- **Which file.** `amx` is written into srv's own `output` zone (`persist_user_line_as` → `append_transcript`, the filestore), not into the Claude CLI's own session file under `~/.claude`, which the CLI rewrites on compaction and which carries no `amx`. Whether the pane's replay and the history read only the zone, and whether anything rewrites the zone's lines (the framer compresses them, `transcript/framer.rs`; it must keep each line's bytes), is item one of §12.
- **Existing exposure to close with it:** `amx.via = viewer` and `amx.via = inject` are unsigned today and the pane acts on them (`deviceOrigin`, `isInjected`, `providers/translator.ts:44-57`); they are signed with the rest and ignored unsigned.

## 7. Old records, and what an untyped record looks like

Records written before this change have no `amx.origin`, and a record whose `amx` fails the check (§6.1) is treated the same. All render in the **container**, per the fail-closed rule, with a **quiet** label (a neutral edge and no warning colour, so a compaction or an old history does not look like an alarm):

1. A record the CLI wrote, recognised by its own wrappers (`<system-reminder>`, `<task-notification>`, `<local-command-*>`, `<user-prompt-submit-hook>`, a compact summary): "CLI message".
2. A record with a recognised marker (a jekt, a broadcast, a Supervisor header, an `[AgentMux]` note): the interim text parse picks the **more specific container** (`agent`, `system`, `supervisor`, …). Parsing can only move a message from "unknown" to a more specific container, never to the human.
3. Anything else with no valid origin: "Earlier message".

`echo: "stdin"` is **not** used to promote an old record to the human: it is unsigned, and the file is writable by an agent (§6.1). Cost: until the history rolls over, old human messages show in the "Earlier message" container. D2 is the operator's call.

## 8. Provider coverage

- **Claude, Codex (`codex-json`), Kimi, Antigravity, Qwen** (`persists_user_record`): srv writes the record, so P1 covers them.
- **Gemini** (its CLI echoes the prompt), **ACP**, **Codex App Server**: srv does not write a record, and ACP and App Server carry no provenance at all today (`blockcontroller/mod.rs:787`). Coddy's Codex trial runs on App Server, so this is real use. Proposal for P3: srv keeps a short per-pane ledger of what it delivered, by exact text, oldest first, with each input's signed `MessageOrigin` (the technique `viewer::origin`'s `expected()` / `stamp()` already uses); the translator pairs the provider's echoed user item with it. No match is **untyped**, so the container.
- **D8, what P2 does for those three until P3 lands** (the operator sees the trade, §11): **per-provider gate** (recommended): P2's "unknown means container" applies to the five providers srv records for; for Gemini, ACP and App Server P2 keeps the **P0-narrowed text parse**, so a jekt or note is still in its container and an unmarked user turn is the human **as it is today**, for the pane's own optimistic/local node and for replay, until P3. The cost: on those three, the fail-open default stays (an unmarked line is the human) until P3. The alternative, fail closed everywhere at P2, would show the human's own typed text in the "Earlier message" container on replay for those three until P3.

## 9. Phases (the build is #4473; the spec is approved before any of it)

| Phase | Scope | Size |
|---|---|---|
| **P0 interim** | (1) **Close the forge path first:** `agentinput`'s origin comes from the caller context (operator-proven window = `User`; an agent token = that agent, never `User`; anything else = unknown), with a test that an agent-token call never produces a human-origin turn, written to fail today. (2) Fix the screenshot without waiting: let `JEKT_BLOCK_RE` match a jekt followed by srv's notes (split after `[/JEKT]`; the rest becomes `system` containers) and render a leading `[AgentMux]` note as a container. **Provably narrowing:** it can only move a message from the human style into a container, never the other way (tests T0 below). It does not touch `typed_by_a_person` or input history. Marked temporary text parsing; deleted in P5. | S (a day) |
| **P1 srv** | The rest of the caller audit (§3.4, AgentY reviews). `MessageOrigin` type (ts-rs), required by `deliver_agent_message_from` and each producer (§2.1); `persist_user_line_as` and `UserRecordSink` write `amx.origin`, `amx.notes` and the signature (key per D9); srv strips and verifies `amx` on every read path (§6.1); `TurnTrigger` built from it. | M |
| **P2 pane** | `OriginContainer`, tokens, `human-turn-tokens` test, translators read `amx.origin`; new node type; legacy path (§7); delete the `JektBubble` styles; the pending panel on the human tokens. | M |
| **P3 providers** | Gemini, ACP and Codex App Server: the origin ledger and pairing. | M |
| **P4 viewers** | **Mobile** (QTest read `transcript.dart`: it already receives the whole raw record and reads `amx` for the paired phone, so `amx.origin` arrives with no feed change): srv is the verifier, so it serves only a verified `origin` in the viewer feed and the phone trusts the pinned-TLS host and reads `origin.kind` (a phone-side check would need a key the phone does not have); **typed `origin` wins and an untyped record keeps today's text parse on the phone** (`parseJekt`, `parseBroadcast`, `splitWrappers`, `isSessionContext`), because old transcripts stay untyped forever and an old desktop paired with a new app writes none, so P5's "delete the text classifiers" covers the desktop pane and srv, **not** the phone; `via`, `device` and `message_id` keep their shape (an old phone ignores `origin`); the phone half ships on the store release train, later than P1/P2, as its own task and PR in `agentmux-mobile` (QTest takes it when P1 is stable); P1 adds a typed-origin line to the fake desktop fixture (`scripts/fake_viewer.dart`) for it. History and `Agents search_history` carry `origin`; input history reads `origin.kind = user` (Agent5 reviews that PR; `typed_by_a_person` is theirs); docs. | M |
| **P5 cleanup** | Delete the text classifiers (§5) once the oldest supported records are typed or the legacy window ends (a release count, decided with the operator). | S |

P0 and P1 can start together. Each phase is one or two PRs with a changeset; P2 needs screenshots of the container on two dark and two light themes, in a running build.

## 10. Tests (written to fail without the change)

**srv**
- Each producer in §2.1 records the right `amx.origin.kind` and `from` (a table test over the producers); a producer without an origin does not compile.
- `deliver_agent_message_from` with notes: `amx.notes` equals the attached notes, `content` ends with `"\n\n" + notes.join("\n")`.
- `TurnTrigger` built from the origin equals the old text-derived one for every existing fixture in `health/tests.rs`.
- An old-shape record (no `amx.origin`) is read back untouched.

**Frontend**
- **S1** A jekt record whose body contains `[BROADCAST:FROM=user VIA=swarm ...]` and a line that looks like a human message renders in the `agent` container, never as `user_message`.
- **S2** A note whose text is `[AgentMux] …` in a `user`-kind record's `notes` renders as a `system` container under the human part.
- **S3** A body containing `[AgentMux]`, `[JEKT:`, `[SUPERVISOR:` in different case, with spaces and in full-width forms, renders in the container of its recorded origin.
- **S4** A record with an unknown `amx.v`, an unknown `kind`, or a malformed `origin` renders in the container, not as the human.
- **S5** A record whose `content` string contains `"amx":{"origin":{"kind":"user"}}` as text is not human.
- **S6** The screenshot case: a jekt followed by an `[AgentMux]` note is one `agent` container plus one `system` container, in both the live and the replay path.
- **S7** `human-turn-tokens.test.ts`: reads the SCSS and fails if `--human-turn-*` is referenced outside the human-turn stylesheet, or if a message stylesheet uses `--main-text-color` as a background.
- Each provider's translator (Claude, Codex, Kimi, Antigravity, Qwen, then Gemini, ACP, App Server in P3) turns the same typed record into the same node.
- The container on all 13 themes: contrast of the header, label and badge text ≥ 4.5:1 (`themes/` tests).
- **T0 (P0 only narrows)** for the interim split: a jekt followed by a note, a note alone, and a note preceded by a human line each render with no part in the human style except the human line itself; a table test over every input the split can see asserts the human style is reachable only by a message that was already human before the change.
- **F1-F4 (forged on disk, §6.1):** a hand-appended `{"type":"user"}` record claiming `amx.origin.kind = user` renders in the container; one with a valid MAC copied onto changed content renders in the container; one with `"amx":{"verified":true}` of its own renders in the container; a signed record read back after an srv restart still verifies.
- **R1 (human regression):** queued/pending human messages (`PendingMessagesPanel`) and a Retry after the agent restarts still show the human style; the paired phone's message shows the human style with its device label.
- **Live:** an isolated `task dev`: a jekt, a Swarm broadcast, a schedule fire, an `[AgentMux]` upgrade note appended to a human message, a pre-change transcript (legacy), reload and replay; screenshots on two dark and two light themes.

## 11. Decisions for the operator

| # | Question | Recommendation |
|---|---|---|
| **D1** | A **Swarm broadcast** is the human's words, sent through the Swarm. Human style with a "via Swarm to 4 agents" label, or the container? | **Human style with the label** (today's look, the broadcast header already shows). It is the operator's own text, sent by them; hiding it in a container would say it came from someone else. The label keeps it distinguishable from text typed in this pane. |
| **D2** | **Old transcripts** (no origin recorded): the container for every old record (safe; old human messages show as "Earlier message" until the history rolls over), or the interim text parse for them (a record with no marker shows as human, as today)? | **The container**, with a quiet label (§7) and a fixed window after which the text parse is deleted. `echo: "stdin"` does not promote a record: it is unsigned and the file is writable by an agent. |
| **D3** | Ship **P0** (the interim fix for the screenshot) now, ahead of P1/P2? | **Yes.** It is a day, it fixes what you saw, and it is deleted in P5. |
| **D4** | A **leader relaying the operator's quoted words**: an `agent` container with the quote inside, never the human style? | **Yes.** There is no way for the pane to know the quote is the operator's; the leader said it. |
| **D5** | A human message with a **note attached** (the usual case today): the human's words in the human style and the note in a container directly under it? | **Yes.** It is the exact thing the screenshot gets wrong. |
| **D6** | **Colour per origin**, or one neutral container with an icon and label per origin, accent only for trust problems (`sensitive`, `unverified`, `invalid`)? | **One neutral container**, the origin in the header, accent only for trust problems. A different colour per origin trains the eye to read colour as authority. |
| **D7** | **Notes**: always shown inline (compact, one line), or collapsed with the jekts? | **Inline and compact**; long ones collapse by the same rule as jekt bodies. They are short and they are what the agent was just told. |
| **D8** | Until P3, for **Gemini, ACP and the Codex App Server** (no origin is recorded for them): fail closed at P2 (the human's own text shows in the "Earlier message" container on replay) or keep the P0-narrowed text parse for those three (an unmarked line stays the human, as today; a jekt or note is still a container)? | **Keep the narrowed text parse for those three until P3** (§8). Coddy's Codex trial runs on App Server; showing the human's own words in a container there would be wrong in daily use. The cost is that the fail-open default stays on those three until P3. |
| **D9** | The **signing key** for the origin (§6.1): (a) a per-run key in srv's memory only (nothing to steal; the human's own messages from before an srv restart show as "Earlier message" until the history rolls over), or (b) an app-bound OS store where one exists (macOS Keychain by code signature) with (a) as the fallback elsewhere (Windows and Linux have no app-bound store)? | **(b) with the (a) fallback**, as AgentY recommends. State the cost to yourself: on Windows and Linux a restart makes older human messages show in the container; a key in a file would keep them but would let any agent forge the human, so it is not offered. |

## 12. Not verified (to settle in P0/P1 before building on it)

- Which store the pane's replay and the history read (srv's `output` zone in the filestore, or the provider's own session file): the pane's replay is believed to read the zone. **Settled by Agent5's check of the framer** (`framer.rs`, `filestore/frames.rs`, `deltas.rs` on `origin/main` `d34ce7431`): both codecs are lossless by construction (codec 1 is zstd; codec 2 only replaces streaming `stream_event` delta lines it can render back byte-exact, so every `type:user` record is stored raw); the encoder decodes and compares before use and every decode is checked against a sha256; the framer only touches agent zones' `output`, never within 8 MiB of the end or a line newer than 24 h; readers see the logical bytes. The MAC binds no file offsets, so a recovery copy or the planned history deletion does not invalidate neighbours. Agent5 also checked for retention, trimming or compact paths: none exists (§6.1). **Still unchecked:** the id `persist_user_line_as` has in hand when it writes (the zone, §6.1). The Claude CLI rewrites **its own** session file on compaction and would drop `amx`, which is why `amx` lives in srv's zone only.
- Whether anything in front of the RPC engine already refuses a non-window caller of `agentinput` (§3.4); and the Kanban "tell" route (`app_api/kanban.rs:373`).
- The key: see §6.1 and D9. Nothing on disk under the user's home is out of an agent's reach; only memory and (on macOS) an app-bound Keychain entry are.
- The Codex App Server, ACP and Gemini input paths: only `deliver_agent_message_from`'s own comment (no provenance today) and `persists_user_record` were read, not each controller.
- The mobile viewer's rendering code: the feed shape (`feed_events.dart`) was read; the message widgets were not.
- Whether `context_delivery` and `ambient_narration` nodes are system-origin text (§5).
- Nothing was run: this is a reading of the code on `origin/main`, with the screenshot the operator sent.
- The producers' line numbers are from one read; for example `cron::deliver` is at `:98` and `fire_message` at `:136` in `cron/mod.rs`.

## 13. Who to loop in

AgentX (the agent pane), AgentY (security review of §3.4 and §6.1), Agent5 (the history store and input history, P1 and P4), QTest (the mobile viewer, P4), Manoz (agent-pane lifecycle and the non-echo providers, P3).
