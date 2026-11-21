# SPEC: Remote panes: open an agent that runs on another computer, see the same conversation live, and drive it, under a red "Remote" banner

**Date:** 2026-11-20
**Status:** active: approved by the operator on 2026-11-20 with every recommendation in section 11 (D1 to D15). Phase 0a (section 9) is built in the pull request that adds this file (build #48246); nothing else is built yet, and phases 0b to 4 remain.
**Kanban:** #48245 (this spec), #48246 (the build).
**Written against:** `origin/main` at `ed9e1974ff` (a few facts on `a2d2133de0`, the commit before it). Every `file:line` below was read on that commit; spot-check one before relying on it.
**Related, read first:** `SPEC_AGENT_PANE_CROSS_CHANNEL_LAN_WAN_SYNC_2026_08_21.md` (an earlier draft of "mirrored panes", which this spec narrows to what was asked and moves onto the code that exists now), `SPEC_AGENTS_OPERATE_OTHER_MACHINES_2026_10_30.md`, `SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md` §8.3 (the computer-to-computer pairing this spec builds on), `SPEC_LAN_KEY_CONTINUITY_2026_10_29.md`, `SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md`, and the live-facts spec (#48221, #48222), a draft whose shape is proposed.

## 0. Summary

**The ask, as a requirement.** A person on one computer clicks an agent that runs on another computer (a paired one on the LAN) and gets a pane showing the same conversation as the one on that computer: its history first, then every update as it happens, with its state (working, waiting, a question open, messages queued). The person can type into it, answer its questions and stop it, as if sitting at that computer, and every update shows on both computers. The pane carries a red banner naming it "Remote" and the host, for as long as it exists, so nobody takes it for a local agent. The agent keeps running where it is; nothing moves.

**The design in seven lines.**

1. **A second view, never a second copy.** The agent, its zone, its process and its input queue stay on the host. The viewing computer reads the host's output zone through the host and shows it with the same pane code, in a mode that writes nothing back to the host (section 7.1).
2. **The transport is the paired-device channel that already exists, not the LAN router.** The host's viewer listener (HTTPS, the viewing computer pins the host's certificate, a per-device token, a per-device signing key) already serves a live conversation feed and a signed "send" route to paired phones, and already carries computer-to-computer reads for Tower. A paired computer is one more device on it. The LAN router is plain HTTP and carries no conversation (section 3).
3. **The host agrees, per pair, off by default, with two separate grants.** Pairing starts on the host (a one-time code the host's person shows), and gives **watching**. **Driving** is a second, deliberate switch on the host, with plain words about what it allows (section 5.1). Both are revocable at once.
4. **The viewing person acts, not an agent.** Driving is a signed request from the viewing computer for a person's click in a window. An agent on the viewing computer cannot reach it. On the host the message is recorded as "&lt;user&gt; on &lt;computer&gt;", never as a local keystroke and never as the agent.
5. **The host always shows who is looking and who is driving**, on the agent's pane and in the status bar, and in the Audit.
6. **Honest about what a viewer sees.** The conversation record has no redaction today, so a viewer sees what the host's own pane shows, tool output included. The grant says so in plain words, and a filter on the host drops the few things that must never leave (section 5.3).
7. **Small steps.** Phase 0 is two small safety fixes that stand alone and ship first. Then the host side and the viewer side, in parallel against one shared test suite, then input.

**Decisions for the operator:** fifteen, section 11, each with a recommendation. The ones that change what you see or what is allowed: D1 (the transport), D2 and D3 (pairing and the two grants), D4 (a remote pane opens watch-only), D5 (what a viewer may not do), D6 (secrets), D9 (the banner), D15 (what the agent reads).

## 1. What exists (code-verified)

| Area | Fact | Where |
|---|---|---|
| Output zone | One zone per agent, `agent:<defId>:current`, file `output`: raw provider lines. The block keeps its own copy under `b:<blockId>` unless `agent:transcriptsinglecopy` is on. Srv stores and serves it; it does not render it | `crates/srv-core/src/backend/agent_session/zone_naming.rs:27`, `transcript/append.rs:24-28,285-313` |
| Reading history | `agent:open_tail` (newest 64 KiB, snapshot overlay); `blockfile:read_range` (offset is a **line index**, at most 10,000 lines, must carry the zone's generation id); `blockfile:line_count`. Every line is origin-vetted on the way out | `crates/srv/src/server/app_api/open_tail.rs:32,48`, `blockfile.rs:198-258`, `blockfile_read.rs:29-39` |
| Live updates | MPS event `blockfile`, scope `block:<id>`, with the line position; the pane fills a gap with `read_range` (at most 5,000 lines); the stream inbox (#7893) hands frames on once per animation frame | `crates/srv-core/src/backend/mps.rs:55`, `transcript/append.rs:349-393`, `frontend/app/view/agent/useAgentStream.ts:174-186,364`, `stream-inbox.ts:76-151` |
| Parsing | In the frontend: the provider translator and `ClaudeCodeStreamParser` for live lines, `parseHistoryLines` for history. Srv parses only to index, count and sign | `frontend/app/view/agent/stream-pipeline.ts:14,24`, `parseHistoryLines.ts:69` |
| **The viewer listener** | A separate listener, **HTTPS** (rustls; a self-signed certificate; the client accepts exactly one certificate by fingerprint, handshake signatures still verified). Routes need `Authorization: Bearer amxv_…`: neither the LAN key nor the instance key opens them, and the token opens nothing else | `crates/srv-core/src/backend/lan/listeners.rs:451`, `viewer/cert.rs:28`, `viewer/client.rs:1-30`, `crates/srv/src/server/http_viewer.rs:1-30` |
| **Pairing** | The host shows a one-time code (10 base32 characters, once, 120 s, only the newest works; 5 wrong codes a minute per address get 429; 10 wrong in a row cancel every code). Redeeming it returns a token (stored hashed) and registers the device with an optional device key. A paired device is listed in Settings with **Can send** (off by default) and can be revoked, which closes every open feed of that device | `crates/srv-core/src/backend/viewer/pairing.rs:1-12`, `http_viewer.rs:146-216`, `storage/viewer_devices.rs:90-94`, `viewer/mod.rs:66,217-260`, `object_schema.rs:661,943` |
| **A computer already pairs this way** | Tower pairs this computer as a viewer device of another one, from the other's pairing link (address, port, certificate fingerprint, one-time code); the token goes to the OS keychain, the fingerprint and address to `tower-peers.json`; a moved peer is found again by fingerprint. The other side answers only if its user turned sharing on | `crates/srv-core/src/backend/tower/peers.rs:1-30`, `http_viewer.rs:298` |
| **A live feed that already exists** | `GET /agentmux/viewer/agents/:name/feed`: SSE, a snapshot (the last 50 turns or 256 KiB), then each append, resumable with `Last-Event-ID` = `gen:line`, status events, a 15 s heartbeat; a device 1 MiB behind is dropped; a frame over 64 KiB is cut to a 2,048-character head. Origin vetting runs on the host before a line leaves. An agent can be hidden from paired devices (one flag per agent, not per device) | `http_viewer.rs:54-57,72,398-445,688-700`, `viewer/feed.rs:30-40,334-340`, `viewer_devices.rs:125-147` |
| **A signed send that already exists** | `POST /agentmux/viewer/agents/:name/input`, text only. It needs the owner's grant (Can send), a signature by the device's key over method, path, time (within a minute) and the body hash (`amxv-sig-v1`, ECDSA P-256 or Ed25519), a `message_id` so a retry delivers once, at most 30 a minute and 64 KiB. The agent gets **a plain user turn with no label**; the stored record carries `amx: {via: "viewer", device, message_id}` and the desktop pane shows "From &lt;device&gt;". There is no route for answers, stop, settings or attachments | `http_viewer_input.rs:1-125`, `viewer/signing.rs:1-25`, `viewer/origin.rs:1-30` |
| The input path | `agentinput` → `composer_origin` (refuses an agent's connection; needs an operator-proven connection or a headless srv) → a signed user record. Appends are serialised per block | `crates/srv/src/server/agent_handlers/input.rs:274,359-400`, `composer_origin.rs:28-44`, `transcript/append.rs:64,89,116,399` |
| Answers and stop | `agentanswer`, `agentcancel` and `tooldecision` take the call's context and **ignore it**; only the window's pane code calls them (`useAgentQuestions.ts`, `useAgentDecisions.ts`, `question-panel-timeout.ts`). Stop is `controllerinput` SIGINT, which is **not** in the always-recorded list | `crates/srv/src/server/ws_register.rs:317,547-551,886-906`, `crates/srv-core/src/backend/rpc/action_commands.rs:24-30,83` |
| The guard to reuse | `ctx.operator_only(why)`: the connection must be the AgentMux window, proven through the host (or a signed-in browser on a headless srv), and not registered as an agent | `crates/srv-core/src/backend/rpc_types/misc.rs:251-263`, used at `app_api/pins.rs:107`, `agent_model.rs:353,371`, `agent_spend.rs:105`, `audit_query.rs:89` |
| Pane state | Working/waiting/idle: srv (`AgentState`). Model, effort, permission mode, status line: block meta. Composer draft and sent history: srv, per agent. **Queued messages: srv memory, not exposed.** A pending question is srv memory; its text is in the transcript. Tool permissions are auto-allowed today (`should_route_to_decision_panel` is hard-coded false) | `crates/srv-core/src/backend/agent_state.rs:62-79`, `agent_runtime.rs:4-23`, `blockcontroller/persistent/mod.rs:397,526,1017-1019`, `input.rs:983` |
| Attachments | Ingested **by host path**; the composer sends an id and a name; a send copy is at most 5 MiB; the stored user line lists the copy's host path. No peer route serves bytes or accepts an upload | `crates/srv-core/src/backend/rpc_types/attachments.rs:14-19,162-165`, `attachments/process.rs:29`, `prompt.rs:225-236` |
| **The LAN router** | Plain HTTP. Routes: remote op, inject, agent, agent-names, holding, fleet, fleet events, health. The broadcast `lan_key` admits a request to be **checked**; it never authorizes one. Remote operations are signed (integrity), not encrypted | `crates/srv/src/server/routes.rs:90-96,210-240`, `auth.rs:192-225`, `remote_ops/op.rs`, `receive.rs:162-284` |
| Audit | Always recorded: `agentinput`, `agentanswer`, `agentcancel`, `agentstop`, `agent.send`, `tooldecision`. A per-day index keeps scrubbed summaries, never a tool result | `rpc/action_commands.rs:24-30`, `audit_activity.rs:6-13` |
| The Swarm's LAN rows | Listed with liveness and a seen-ago time, stale after 60 s. A remote row has **no click**. `Agents open` with `host` opens the agent in a tab **on the peer**, not on the caller's screen | `crates/srv-core/src/backend/swarm_remote.rs:40,289,316`, `frontend/app/view/swarm/swarm-view.tsx:112-174`, `crates/srv/src/server/app_api/agent_open_remote.rs:1-9,80-90` |
| Banners on the host | None for a viewer. The nearest is the sensitive-operation countdown banner | `frontend/app/view/agent/sensitive/SensitiveBanner.tsx` |

**Not found:** a viewer route for history paging beyond the snapshot, answers, stop, settings or attachments; a server copy of queued messages; any redaction on the transcript write path (the helpers are called from the continuity, browser-terminal and login code, not the transcript); a per-device agent scope (hiding is per agent, for every device). **Not verified:** whether an image block in provider output is stored in the zone; whether a Vault value or the agent token can appear in tool output; whether the hidden memory re-injection turn is persisted in the zone (phase 0 settles all three). **Draft, not built:** the live-facts operation (#48221, #48222).

## 2. Principles

1. **One source of truth.** The host's zone, queue and agent. The viewer holds a bounded in-memory window of it and writes nothing to the host's stores.
2. **The host decides, per pair.** Nothing is shown or accepted without the host's grant for that one computer, and a grant for watching is not a grant for driving.
3. **A person acts, an agent does not.** A remote pane is a person's act on another computer.
4. **Proof and secrecy, not location.** Everything between the computers rides TLS pinned to the host's certificate; a send is also signed by the viewing computer's own key.
5. **Attribution on both sides.** "&lt;user&gt; on &lt;computer&gt;" in the host's conversation, its Audit and the viewer's Audit.
6. **Typed, not text.** The host acts on structured requests; banner and indicator text is built from fields, never from the other side's words.
7. **Nothing is silently queued.** A send that cannot reach the host fails visibly.
8. **Say what is shown and what is allowed.** The viewer sees what the host's pane shows; driving is command execution when the agent is native. The consent text and the banner say so.

## 3. A. Transport and what the viewing computer needs

**Decision D1: the viewer listener, not the LAN router.** The conversation carries tool output and the contents of files an agent read, so the channel must be confidential as well as authenticated. The LAN router is plain HTTP and its signed operations give integrity only; an SSE route there would send conversations in the clear to anyone on the network. The viewer listener is HTTPS with a certificate the viewing computer pins by fingerprint (the handshake signature is still checked, so the peer must hold the key), and its token, its signed send, its feed engine, its origin stamp, its device list and its revoke already exist and already carry computer-to-computer reads for Tower. Using it also satisfies "one transport" (the paired-device channel) and "keys pinned on both sides" (the viewer pins the host's certificate; the host pins the viewer's device key at pairing). What it costs: the LAN router's remote-operation envelope is not reused, and the routes below are added to the viewer listener.

| Need | Source on the host | Route (on the viewer listener, token required) | Stored on the viewer? |
|---|---|---|---|
| History, newest first | the output zone: `open_tail` and `read_range` | the feed's snapshot (50 turns or 256 KiB), then `GET /agentmux/viewer/agents/:name/history?before=<gen:line>&limit=` for older pages of at most 2,000 lines (new) | In memory only, in a ring of at most 20 MiB per remote pane (oldest evicted, reloaded on scroll). Never on disk |
| Live updates and state | the `blockfile` append events; `AgentState` and block meta | `GET .../feed` (exists): appends, `status` events, resume by `gen:line`, 15 s heartbeat | Same ring |
| Open question and queued messages | srv memory (`pending_questions`, `pending_send_messages`) | new `pending` event on the feed: the open question's id and kind, and a count of queued messages (the text of a queued message is **not** sent: it is the host person's draft until sent) | No |
| Attachments and images | a stored attachment, by id | `GET .../attachments/:id` (new): one attachment the feed names, images and text only, at most 5 MiB, never a path | Memory only, with the ring |
| Name, colour, seat | the agent's definition and block meta | in the feed's first event | No |

**Where it lands on the viewer.** The viewing computer's srv terminates the host channel, so the token, the device key and the reconnects live in one process, and every window of the viewer shares one connection. It keeps the ring and serves its own windows through three small calls shaped like the ones the pane already uses on the host: tail, range, live events. The pane gets a **source adapter** in place of the local zone subscription; its parser, renderer and composer are the ones that exist. A torn-off window and a second window on the same remote agent read the same ring. The viewing computer keeps, per host: its address, port and certificate fingerprint (as Tower does), the token in the OS keychain, and its device key in the keychain.

**Why not write the lines into a local zone** (so the pane reads it as any local pane): it would persist the host's conversation on the viewing computer (section 5.3 rules that out), and the file store has no memory-only zone today.

**Opening a feed is signed, not only authenticated** (condition of the security review). A feed open for a paired computer carries `X-Amxv-Timestamp`, `X-Amxv-Nonce` and `X-Amxv-Signature` over `amxv-sig-v2`, the method, the path, the timestamp, the nonce and the **host's certificate fingerprint** (the host's key id), signed by the viewer's device key; the host accepts a timestamp within a minute, remembers the nonce for three minutes, and refuses a replay. The stream is bound to that device: it closes at revoke (the lease already does), when the device's key is replaced, when the grant is narrowed, and on the host's own restart; a changed host certificate makes the viewer refuse the connection and ask to pair again, never silently.

## 4. B. What the person can do, and what is refused

Every action is a request from the viewing computer, for a click or keystroke of a person in a window there. The host checks the token, the signature, the grant and its mode, then runs **the same code path the host's own pane uses**, with the origin stamped as remote.

| Action | Watching | Driving | Route and notes |
|---|---|---|---|
| Read the conversation, state, cards, attachments | yes | yes | the routes of section 3 |
| Type and send a message | no | yes | `POST .../input` (exists; the device must have Can send, which is what "driving" is). The composer's text is local to each computer; the host orders sends by its own per-block queue |
| Stop the turn | no | yes | `POST .../stop` (new), the pane's own interrupt; recorded in the Audit |
| Answer or dismiss a question card | no | yes | `POST .../answer`, `.../cancel` (new) |
| Drop an attachment | no | yes | `POST .../attachments` (new, phase 3): bytes at most 5 MiB, stored on the host under its own data folder, an id returned; a path is never sent in either direction |
| Change model, effort or permission mode | no | yes | `POST .../settings` (new), only those three, through the host's own setter |
| Approve a tool permission | no | **no in v1** | Tool permissions are auto-allowed today (no panel exists). If a permission panel is built, answering it remotely needs its own scope, "may approve", granted separately (D5) |
| Change the agent's account, Vault grants, env, seat or name; restart, close or delete the agent | no | **never** | These stay on the host |
| Open a file link | no | **no** | A link is a path on the host. The viewer shows the path as text and opens nothing |
| Run a command on the host, read a file, use another pane or agent of the host | no | **never** | Not part of the pane's surface. (Driving a **native** agent is nonetheless command execution on the host by the agent: section 5.1) |

**Who is "the person".** The viewing computer signs a send, stop, answer or setting only for a call its srv has proven to come from a person's window (the operator-proven connection `composer_origin` already requires), and a registered agent's connection is refused there in every mode, as it is for a local composer. An agent on the viewing computer cannot send, stop or answer through a remote pane.

**How a remote approval is proven to be a person** (D5, for when permission panels exist): the click is a call from an operator-proven window on the viewer; the viewer signs it with its device key; the host verifies the "may approve" grant, that the question id is open and was asked of this agent, and records "approved by &lt;user&gt; on &lt;computer&gt;". The agent's own text is never part of the decision.

**What the host records.** The host writes the sent message with `amx: {via: "viewer", device: "<computer>", user: "<user>", message_id}` and `origin.kind = user`, the phone's form extended with the user name. Both panes render it as the human's message labelled "from &lt;user&gt; on &lt;computer&gt;". The computer is proven (its key signed the request); the user name is the computer's own claim, so the label says so in the Audit row ("claimed by the paired computer"). The signature on the record is the host's, as for every record; a viewer cannot write a record. The Audit rows name the actor `<user>@<computer>`.

**What the agent reads** (condition of the security review). **Parity with the phone: exactly the text that was typed, as a plain user turn, with no label, no marker and no note**, delivered the way a Swarm broadcast turn is (a running agent reads it at its next step, mid-turn included; an idle or stopped one starts a turn). The agent therefore cannot tell a message from the viewing computer from one typed on the host. The label, the device and the user exist only in the stored record, the panes and the Audit. A stop and an answer reach the agent as the host's own would. D15 asks whether to add a label the agent can read (recommended: no, because an agent's text is an instruction channel and a label an agent could be told to ignore adds nothing a person can rely on; the host's "being driven" strip is the human-facing signal).

**The two safety fixes that stand alone and ship first (phase 0).** (a) `agentanswer`, `agentcancel` and `tooldecision` are gated by `ctx.operator_only(...)`; (b) a stop is recorded in the Audit when it comes from a paired computer, and `controllerinput` with an interrupt is recorded for the window too. See 5.5 for the justification and what changes for agents.

## 5. C. Security

### 5.1 Who may watch or drive: two grants, on the host

- **Watching is granted by pairing, on the host.** The person at the host opens the Host panel's "Pair a device" (it needs LAN discovery on, as for a phone), and shows the one-time code or link to the other computer, where the person enters it. That is the host's agreement: nothing is offered, and nothing can be requested, until the host's person does this. **Off by default, per pair.** The new device appears in Settings › Paired devices as a **computer**, with its name, certificate-pinned fingerprint of the host side and the device key's fingerprint on the viewer side (to compare), when it paired and last read, a **Revoke** button, and an "Agents it may see" list.
- **Driving is a second switch on the same row: "Can drive".** The phone's "Can send" is this switch, so a computer that is paired but not allowed to drive can only watch, even if it sends a signed request. The switch has its own text, in plain words, which the host's person reads when turning it on:
  - for a **native agent**: "&lt;computer&gt; will be able to send messages to this agent, and a native agent runs commands on this computer. This lets that computer cause commands to run here, as this agent."
  - for a **container agent**: "&lt;computer&gt; will be able to send messages to this agent. The agent can reach what its container can reach: &lt;the mounts and network access the agent has&gt;."
  - and the same two sentences are the first line of the "being viewed" strip while a computer is driving that agent (5.2).
- **Per-agent scope** is offered in the same panel: all agents, or chosen ones (phase 4; until then the existing "hidden from paired devices" flag, which is per agent for every device, applies). A grant for chosen agents refuses the others.
- **Only a person decides.** Pairing, turning Can drive on, and revoking are calls that need the window's operator-proven connection (`ctx.operator_only`). An agent, the super agent and a remote request cannot pair, grant or widen. This is the guard the fleet rules keep: it stops a forged consent.
- **Both pins.** The viewing computer pins the host's certificate fingerprint from the pairing link (a new host certificate is refused and the pane asks to pair again), and the host pins the device key given at pairing (a changed key stops the view and asks again). Neither is ever moved silently (the rule of `SPEC_LAN_KEY_CONTINUITY_2026_10_29.md`).
- **Revoke at once.** Revoking closes every open feed of that device and refuses the next request; the viewer's pane switches to "Access ended by &lt;computer&gt;" and keeps the last content read-only. Turning Can drive off ends driving the same way and leaves the view open.
- **A grant narrows, never widens.** Watching cannot send even if a signed send arrives. The remote-operations switch and the Tower sharing switch do not affect this channel; this channel has its own switch, "Allow other computers to watch agents here", off by default, and pairing is refused while it is off.

### 5.2 The host always shows it

- **On the agent's pane on the host:** a strip under the pane header, "Being viewed by &lt;computer&gt;" with one entry per viewer, and "driving" beside one that has sent in the last two minutes, with the native or container sentence of 5.1 for a driver. It follows the chrome zoom like the pane's other controls and can be hidden by neither the agent nor the viewer.
- **In the status bar of the host:** a chip, "&lt;n&gt; viewing", whose panel lists the viewers with a **Stop viewing** button each (ends this session without removing the grant). It shows even when the agent's pane is not open on the host.
- **A notice** on the host when a viewer first connects in a session and when one first drives.
- **Audit rows** on both computers for each pairing, grant change, feed open and close, send, stop, answer, cancel, setting and revoke, naming the viewer, the agent and the verification; for a send, the position of the line it produced, never its text (the text is in the conversation, as for any message). The viewer's Audit records the act, not the content.

### 5.3 No new route to secrets

The transcript has no redaction (section 1), so the rule cannot be "the record is clean". It is:

1. **Never sent:** a Vault value; the agent's token or any environment value (the feed is the zone's lines, not the process environment; the env file holds only `vault:<id>` references); another agent's memory (the hidden re-injection turn is filtered out by its marker); a host path in an attachment (the user line's attachment list is rewritten to name and id); inline image bytes in a user record (the writer already drops them).
2. **A filter on the host, in one place** (the feed engine's vetting step, beside the origin vetting): an allow-list of record kinds, the rewrites above, and a best-effort pass using the existing `redact_secrets` helpers over secret-shaped strings in tool output (D6). A test seeds canary values (a Vault reference, an environment value, an attachment path, a re-injection turn, a token-shaped string in a tool result, an image block) and asserts none reaches a viewer. **Phase 1 does not merge without the filter and this test.**
3. **What a viewer does see**, said in the pairing text: everything the host's own pane shows for that agent, including what its tools printed and the files it read. A pairing is in effect "may read this computer's agents' work as I can".
4. **The viewer stores nothing at rest.** The ring is in memory, capped, dropped when the pane closes; no cache file, no search index, no history entry. The pairing record keeps only the address, certificate fingerprint and a token.
5. **No other surface of the host** is reachable from a remote pane: no Terminal, Files, Browser, Vault, Memory, Tasks or another agent. The viewer routes serve only the agent's feed, history, attachment, and the actions of section 4.

### 5.4 Limits, offline, several viewers, containers, WAN

- **Rate limits (host):** sends 30 a minute per device (exists); stop, answer, cancel and settings 30 a minute together; at most 8 feeds per host and 3 per agent; the pairing-code limits that exist; a feed that is not read is dropped at 1 MiB behind and the viewer resumes from `gen:line`.
- **Host offline:** see section 7. The banner says `Remote · <host> · offline`, the last content stays read-only, the composer is disabled with the reason, and a send fails at once.
- **Several viewers:** up to three per agent. Each sees every update. Sends from any of them and from the host's own person are ordered by the host's per-block input queue. No floor control and no typing indicator in v1: each composer's text is its person's own, and the record shows who sent what.
- **A container agent** works the same: the zone and the input path are the same code and the filter is the same.
- **WAN and cloud are not in the first build.** What they need: the instance-certificate proof that remote operations already name for WAN (`Verification::Wan` exists but is not active, `remote_ops/mod.rs:44-46`), a relay carrying the stream end to end encrypted so the relay never sees a conversation, and a pairing step that does not depend on the LAN. A separate spec; the grants, the filter and the banner carry over.

### 5.5 Phase 0: why the three answer calls get an operator-only guard, and what changes

`agentanswer`, `agentcancel` and `tooldecision` answer a question or a permission request that an agent put to the person. Today they accept any caller on the websocket RPC, so a caller that is not the person (an agent's connection) could answer in the person's voice. That is a forged operator voice, the one case the fleet's rules keep a guard for, so the guard is `ctx.operator_only`, the same one that already protects pin changes, model routes, spend caps and the Audit window.

**What changes for agents today: nothing they use.** (1) The only callers of the three are the window's own pane code (`useAgentQuestions.ts`, `useAgentDecisions.ts`, `question-panel-timeout.ts`) through the window's connection, which is operator-proven; no MCP tool, bus frame, script or other crate calls them (searched on `ed9e1974ff`). (2) An agent that drives its own `task dev` through the shortcut API presses the real buttons and keys of that build's window, so the answer still arrives through that window's connection: unchanged. This is also the rule the pane-request fix (#7871) already states in the interface: an agent answering its own request to the person stays refused, and a cancel (Escape) always gets through. (3) A headless srv admits its signed-in browser (operator-proven by sign-in) and an external host with a person on a lease: unchanged. (4) The paired phone never calls them. What is newly refused: a connection registered as an agent, or one that is not the window, calling one of the three. It gets `FORBIDDEN`, as it does for the other guarded calls. A test pins each of those cases.

## 6. D. The interface

**The banner (exact).** A band across the top of the pane, above its header, reading **`Remote · <host>`** (the host's name as `Agents list` shows it; the middle dot is U+00B7), with a link glyph before it. A watch-only pane adds the state word `watching`, a driving one `driving`; while the connection is not live it reads `Remote · <host> · reconnecting…` and then `Remote · <host> · offline`.

- **Colour.** A new pair of tokens, `--remote-banner-bg` (a red) and `--remote-banner-fg`, defined in every theme with a contrast of at least 4.5:1 for the text and 3:1 against the pane's own edge. In high-contrast and forced-colours modes it uses the system's warning and text colours and a 3 px border; it is never colour alone (the icon and the word carry it). The pane's outline and its tab take a red edge too.
- **Never hideable.** It is not part of the header's overflow, has no setting, and no menu or agent can remove it. Closing the pane is the only way.
- **Zoom.** It follows the chrome zoom like the pane's other controls (the `chrome-zoom-inside` approach of #48236), so it is the same size at any pane zoom, wraps to two lines before it clips, and at the narrowest pane truncates the middle of the host name, never the word "Remote".
- **Screen readers.** `role="status"` with "Remote pane. This is &lt;agent&gt; running on &lt;host&gt;. What you do here happens on that computer." and the state word; a change of state is announced once.
- **The header** shows the host, the connection state, and a lag figure only above one second (from the feed's heartbeat times, never from comparing the two clocks). The pane's tab shows a remote glyph and the host's name.

**Opening one.**

- **From the Swarm:** a remote row gets a click and Enter. If this computer is paired with that host, it opens a remote pane in the current tab, in watch mode; if not, it opens a small panel: "To watch agents on &lt;host&gt;, open Host › Pair a device there and enter the code here." The row carries a remote marker and the host's name.
- **From an agent or a script:** `Agents open` with `host` and a new `view: true` opens a remote pane on the **caller's** own window. It always opens watch-only: an agent can put a view on its person's screen, but only the person can turn on driving.
- **Driving is a deliberate click.** A remote pane opens **watch-only**, whatever the host allowed (D4). A "Drive" control in the header, a person's click, enables the composer if the host granted Can drive, and the banner changes to `driving`. One click goes back to watching.
- **Pairing in one step for two computers one person owns** (D3). The host's pairing panel has a checkbox, "This is another computer of mine: let it be watched from here too". When the person at the host ticks it, the link carries `both-ways`; when the other computer redeems the code, its person sees "Also let &lt;host&gt; watch agents here? [Yes] [No]" and, on Yes (a person's click on that side too), that computer shows the same pairing code back through the open channel and the host redeems it. Nothing is automatic: each side is a person's click, and each side still has its own Can drive switch, off. How the host knows the computers are one person's: it does not decide; the checkbox is the person's own statement. When both computers are signed in to the same MuxBus account the checkbox starts ticked (a hint, never an action).
- **After a restart:** the window tab restores the pane as "reconnecting", reconnecting only while the pairing exists, in watch mode. What is restored is the pane's block (host, agent), never the conversation (D7).
- **Colours.** A remote pane is never mistaken for a local one: the banner, the red edge, the tab glyph and a pane background tint unlike any of the pane's own scope colours.

## 7. E. Edge cases and their behaviour

| Case | Behaviour |
|---|---|
| Large history | Tail first (the feed's snapshot), then older pages of 2,000 lines when the person scrolls up, reloaded after eviction from the 20 MiB ring |
| Host restarts or upgrades mid-view | The feed ends; the pane shows "reconnecting" and resumes by `gen:line`; a new zone generation reloads the tail and says so once |
| Agent stopped, deleted, signed out, or out of quota | The pane shows the state from the status event ("stopped", "signed out", "limited until …"); the composer is off when a send would be refused (the phone route's reasons: `needs_login`, `starting`, `not_running`); a deleted agent ends the view with "This agent no longer exists on &lt;host&gt;" and keeps the last content read-only |
| Viewer or host sleeps | A heartbeat missing for 20 s is "reconnecting", 60 s is "offline" (the Swarm's stale threshold); on wake the feed resumes by `gen:line`; a send made while offline fails at once, nothing is queued |
| Clock differences | Signed requests allow a minute either way (the existing rule); beyond it the host refuses with "the two computers' clocks differ by more than a minute". Times in the pane come from the records and the host's own stamps |
| Version mismatch | The host's `hello` advertises `remote_panes: 1`. An older host has no such routes: the Swarm row says "needs AgentMux &lt;first release with this&gt; on &lt;host&gt;". The minimum is the first release with phases 1 and 2. A newer host with an older viewer simply sees no such requests |
| Two windows on the viewer; a torn-off window | One connection and one ring per (host, agent) in the viewing srv; each window is a pane on it; closing the last closes the feed |
| Pane zoom | The remote pane has its own pane zoom as any pane; the banner and controls follow the chrome zoom. Nothing is sent to the host |
| Images and media | Fetched when they scroll into view, images and text up to 5 MiB, shown from memory; a larger one is a placeholder with its name and size |
| An agent whose pane is not open on the host | Readable: the zone does not depend on a pane. The status-bar chip still shows the viewer |
| The host's person and a viewer send at once | Both go to the host's input queue in arrival order; each composer keeps its own draft |
| A question is answered on both computers | The first answer wins; the second is refused with "already answered" and its pane updates from the feed |
| Compaction, a fresh session, a resumed one | They appear in the feed as records; the viewing pane never runs the host's re-injection (7.1) |
| Memory use | One ring of at most 20 MiB per remote pane, at most six remote panes on a viewer; beyond that the open is refused with a message |

### 7.1 The viewing pane writes nothing to the host

Rendering the host's lines through the frontend's live path would, today, fire host-only writes. The viewing pane runs the same parser and renderer with a **remote flag** that turns each of them into a no-op, in one place each, with a test that lists the calls a remote pane may make and fails on any other:

| Host-only write on the live path | In a remote pane |
|---|---|
| `docknodestatus` for each tool node, and `background_task_observe` for a background run | off |
| `ambientnarrate` (a model call) | off |
| `backgroundtaskcompletion` (a durable registry write), and the dock-status write on an orphans scrub | off |
| Memory re-injection on a compaction boundary or fresh session (a hidden `agentinput` plus `memorydelivery`), which can start a hidden turn in the block it names | off. The highest risk of the five |
| `agent:session:write_state` every 30 s, and a controller resync on launch | off. A remote pane never persists a snapshot |

The history path (`parseHistoryLines`) makes no calls, so history is safe as it is. The viewing pane's RPC client is the viewing computer's own srv; it never holds a connection to the host's websocket RPC, only the viewer routes.

## 8. F. Platforms and how it is proven

- **All three platforms** run the same Rust srv and the same frontend. Per-platform checks are visual (the banner, the strip, the chip and the Settings row in each theme, at the chrome zoom minimum and maximum) and LAN reachability (the firewall setup of `SPEC_LAN_FIREWALL_SETUP_2026_10_01.md`; the viewer port is already part of it for phones).
- **Automated, in CI:** two srv instances in one test process on loopback with the real listener and certificate pinning: pairing and a wrong code, watching without Can drive (a signed send refused), Can drive on and off, a revoked device, a changed host certificate, a changed device key, a replayed feed open, the feed's resume and 1 MiB drop, the filter's canary test, the remote-flag call list, and the Audit rows on both sides.
- **By hand, two real computers** (the operator's and one testbed): pair, watch an agent work, turn on driving, send, answer a question, stop; revoke mid-stream; take the host offline. A scripted or real-mouse run follows the host rules: a testbed's custodian is told first; the operator's own computer needs the operator's yes first.
- **Look checks** on macOS and Linux for the banner, the strip, the chip and the Settings row.

## 9. G. Build order, in small pull requests, and who builds them

| Phase | Contents | Tests | Rough cost | Seat |
|---|---|---|---|---|
| **0a** (now, its own PR) | Gate `agentanswer`, `agentcancel`, `tooldecision` with `ctx.operator_only`; record a stop from the window and from a paired computer | The five cases of 5.5 | 0.5 day | platform |
| **0b** | Settle the three unverified secret questions (image blocks, a Vault value or token in tool output, re-injection persisted in the zone) with a canary test; write the answers into 5.3 | The canary test | 1 day | platform |
| **1** (host side) | The "computer" device kind and the host's own switch, the history, attachment and `pending` additions, the signed feed open, the stream filter, the "being viewed" strip and chip, the Settings row with Can drive and its text, the Audit rows, the `hello` capability. **Does not merge without the filter and the canary test** | The two-srv suite for everything read-only; the canary test | 4 to 5 days | LAN, history store, Audit |
| **2** (viewer side), in parallel with 1 | The viewing srv's channel and ring (the paired-computer store generalised from Tower's), the source adapter, the remote flag (7.1), the banner, the Swarm click and the pairing panel, `Agents open view`, tab restore, the offline states | A pane test per state; the call list; look checks | 4 to 5 days | agent pane, Swarm |
| **3** | The stop, answer, cancel, settings and attachment-upload routes, the Drive control, `via: "viewer"` with the user name and its label, the "driving" indicator, the one-step "both ways" pairing | The two-srv suite for each action and each refusal | 3 days | both |
| **4** | Lag figure, per-agent scope per device, the "may approve" scope if a permission panel exists by then | | 2 days | both |

Phases 1 and 2 are built by two agents against the same two-srv suite, which is written first and is what each side's pull request is held to: the host side touches the LAN and viewer code, the history store and the Audit; the viewer side touches the agent pane, the Swarm and the paired-computer store. Each phase is one or two pull requests with tests and a changeset. About three weeks of work end to end, about two with two agents.

## 10. Not decided here

- The live-facts operation's final shape (#48221, #48222): the state fields of section 3 come from it by field once it exists; until then the feed's status events carry them.
- Whether a permission panel will be built for tool permissions; D5 covers the scope if so.
- WAN and the cloud (5.4).

## 11. H. Decisions for the operator

| # | Decision | Recommendation | Cost |
|---|---|---|---|
| D1 | Transport: the paired-device HTTPS channel (viewer listener) for the feed and every action, rather than the plain-HTTP LAN router | **The viewer listener.** Confidential and pinned; its feed, signed send, grants and revoke already exist | in the plan |
| D2 | Consent: nothing is offered until the host's person pairs a computer; watching comes with pairing; **driving is a second switch on the host**, with plain words about native-agent command execution | **Yes**, as in 5.1 | in the plan |
| D3 | Pairing between two computers of one person: a one-step "both ways" option, a person's click on each side, never automatic | **Yes**, as in section 6 | +1 day |
| D4 | A remote pane opens watch-only and "Drive" is a person's click; an agent or a script can open a view, never turn on driving | **Yes** | in the plan |
| D5 | What a viewer may not do: no tool-permission approval in v1; never account, Vault, env, seat, rename, restart, close or delete; no opening of host paths | **Yes.** A separate "may approve" scope only when a permission panel exists | none now |
| D6 | Secrets: a feed filter, a best-effort redaction pass on tool output, and pairing text saying a viewer sees what the host's pane shows | **Yes.** The redaction is best effort and does not change what the text says | +1 day |
| D7 | After a restart a remote pane is restored as "reconnecting" (host and agent only); nothing of the conversation is kept at rest | **Yes** | in the plan |
| D8 | Several viewers: up to three per agent, the host orders sends, no floor control and no typing indicator in v1 | **Yes** | in the plan |
| D9 | The banner: `Remote · <host>` with state words, a link glyph, a new red token pair with contrast rules, never hideable, a red pane edge and tab glyph | **Yes** | in the plan |
| D10 | This channel has its own switch, off by default, separate from "remote operations" (which are on by default). The reason those default on (a person back in the loop) does not apply to a screen of someone's work | **Yes** | none |
| D11 | WAN and the cloud are not in the first build | **Yes**; a separate spec | none now |
| D12 | The minimum version is the first release with phases 1 and 2; an older host shows "needs AgentMux …" | **Yes** | in the plan |
| D13 | The memory ring: 20 MiB per remote pane and at most six panes on a viewer | **Yes**; adjustable from a measurement after phase 2 | in the plan |
| D14 | A host-side "Stop viewing" for one viewer without removing the pairing | **Yes** | in the plan |
| D15 | What the agent reads from a remote send: exactly the typed text with no label, as the phone's | **Parity, no label.** A label in an agent's text is an instruction channel, not a proof; the host's "being driven" strip and the record are the human-facing signal | none |

## 12. Tests (written to fail without the change)

The two-srv suite of section 8, plus: a send in watching mode is refused; the same send from an agent's connection on the viewing computer is refused; the three answer calls are refused for an agent's connection and the window's passes; a revoked device's feed closes within a second; a changed host certificate is refused and asks to pair again; a changed device key is refused; a replayed feed open is refused; a send writes `via: "viewer"` with the computer and the user, and a viewer cannot write a record; the agent's text for a remote send equals the typed text; the feed never contains the canaries of 5.3; the viewing pane calls only the calls on its list; the banner cannot be hidden by any setting, at the chrome zoom minimum and maximum and at the narrowest pane; each offline state shows its banner text; the Swarm row opens a watch-only pane and an agent's `Agents open view` cannot enable driving.
