# SPEC: See what agents say to each other through the CLI's own messaging: one record, one container, live in the pane

**Date:** 2026-10-26
**Author:** Opaz, for Kanban #4863, at the operator's request via AgentY
**Status:** approved 2026-10-26 by the operator ("go with best recommendations", relayed by AgentY): D2 and D3 as recommended in §7. Build is Kanban #4864 (P0, P1; P2 rides the origin spec). Uncommitted by instruction; ships with the code that implements it.
**Operator's words (via AgentY):** "there are messages that agents send each other via the actual claude cli harness, we want visibility into that talking. It would both show in the agent pane in real time when received/sent, and in our messaging auditing/tooling."
**Related:** `SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md` (the typed origin and the one container this spec adds one kind to; its row 17 names "the CLI's own injected text" and leaves it out of scope), `SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md`, `SPEC_JEKT_DELIVERY_STATES_AND_MAILBOX_2026_10_01.md` (delivery states), `crates/srv/src/backend/reactive/audit_store.rs` (the jekt audit file).

## 0. Summary

- **The channel is real and live on a Linux dev machine today.** Every Claude CLI session (our agents') registers a file and a unix socket, and can message any other session of the same config directory with the CLI's own `SendMessage` tool. On charlie 6 sessions were live and listable by each other (verified 2026-10-26, CLI 2.1.288). No transcript on this machine shows it used yet (§1.1), so this is the moment to add visibility, before habits form.
- **Today nobody can see it.** The receiving agent's turn starts by itself: the CLI writes no text to the stream srv reads, so the pane shows an agent answering a message nobody sees (§1.3). The sender shows a plain tool row. Nothing reaches the jekt audit file or any tooling.
- **It can be observed, with one launch flag.** With `--replay-user-messages` the CLI emits the peer message as a typed stream line carrying `origin.kind = "peer"`, the sender's session name, the sender's verified process id, a message id and the body (§1.2, experiment E2). The sender's tool call carries the same message id, so both ends join into one record.
- **Design.** (1) srv reads those lines and the sender's `SendMessage` tool call, and writes one **CLI-native message record** per message: from, to, when, text, delivery state (§3). (2) The record goes into the same audit file and route as jekts, `event_kind = "cli_native"`, kept the same way (forever) (§3.4). (3) The pane shows it in the **one container** of the origin spec with a `cli_native` origin, labelled "CLI-native", never in the human's style, both on the receiving pane when it arrives and on the sending pane when it goes (§4). (4) Everything it cannot see is listed (§6).
- **Decisions for the operator:** five, §7, each with a recommendation. The two that change what you see or keep: whether the audit holds the full text (D2) and whether agents may keep using the native tool at all (D3).

## 1. What exists today (verified on Linux, 2026-10-26)

Everything marked **verified** was run against the pinned CLI (`claude` 2.1.288, `~/.agentmux/shared/cli/claude/2.1.288`) with two throwaway sessions on Haiku in an isolated config directory (`CLAUDE_CONFIG_DIR=/tmp/cli-native-xs/cfg`, credential symlinked), so no agent received anything. The commands are in §8.

### 1.1 The mechanism

- Each session writes `<CLAUDE_CONFIG_DIR>/sessions/<pid>.json` and listens on `/run/user/<uid>/cc-socks/<pid>.sock`. The file holds `pid`, `sessionId`, `cwd`, `name`, `status` (`busy` | `idle`), `kind` (`interactive`), `entrypoint` (`sdk-cli` for ours), `version`, `peerProtocol: 1`, `peerFeatures`, `messagingSocketPath`. The session's children get `CLAUDE_CODE_MESSAGING_SOCKET` and `CLAUDE_CODE_MESSAGING_TOKEN` in their environment (so every agent shell has them). **Verified (file contents, env names).**
- `ListAgents` lists the live sessions of the same config directory; `SendMessage {to, message, summary?, notify_when_idle?}` delivers to one. Names are derived from the working directory plus a random suffix (`opaz-0909m-75`), not the AgentMux agent name. **Verified.**
- Config directories are per identity (`shared/identities/<id>/claude`), so every agent signed in with the same account on a machine can reach every other one. On the test machine that was the 6 live sessions above.
- Usage so far: zero. A search of every Claude transcript readable on the machine for `cross-session`, `"name":"SendMessage"`, `"name":"ListAgents"` and `teammate-message` found no file. The tool is deferred (the model loads it with `ToolSearch`), which is probably why. **Verified.**
- The CLI delivers in its own way: an idle receiver **starts a turn by itself** (E1); a busy receiver gets it **between tool rounds** (E2). It tags the sender's permission mode (`from-mode`); a receiver in a different mode holds it, which is **verified (E3)**: the receiving CLI writes a `system/peer_message_hold` frame (`state: held`, `cause: mode-mismatch`, `message_uuid`, `from_name`, `from`; no pid, no body) and the message is never handed to its model in a headless session. The sender's tool result still says `success: true`, "in that session's inbox". The `[Cross-session delivery notice]` the tool description promises the sender was not reproduced (§9).

### 1.2 Experiments

| | Setup | What the receiver's stdout (what srv reads) shows | What the receiver's on-disk transcript shows |
|---|---|---|---|
| **E1** | Sender calls `ListAgents`, then `SendMessage` to an idle receiver. No replay flag. | Nothing with the text. `command_lifecycle {state: started}`, a second `system/init`, the model's reply `READY2`, `command_lifecycle {state: completed}`. The `command_uuid` is the message's uuid. | `queue-operation enqueue` with `<cross-session-message from="uds:/tmp/cc-socks/<pid>.sock" from-name="send-61" from-mode="bypass">…</cross-session-message>`, then a user record "Another Claude session sent a message: …" plus a note that it was not typed by the user. |
| **E3** | Receiver launched with the exact args srv uses for Claude panes (`--permission-mode default --permission-prompt-tool stdio`, plus `--replay-user-messages`); sender in `bypassPermissions`. | After the receiver's first `result`: `{"type":"system","subtype":"peer_message_hold","state":"held","message_uuid":…,"lane":"socket","from":"uds:/tmp/cc-socks/<pid>.sock","from_name":"send-02","cause":"mode-mismatch"}`. No user line, no turn, no `command_lifecycle`. | (not read) |
| **E2** | Receiver is mid-turn (three `sleep 12` Bash calls, with `--replay-user-messages`). Sender sends once during the second. | A **user line** between two tool results: `type:user, isReplay:true, isSynthetic:true, message.content = "<cross-session-message from=… from-name=… from-mode=…>\n<body>\n</cross-session-message>"`, and `origin: {kind:"peer", from:"uds:/tmp/cc-socks/<pid>.sock", verifiedPeerPid, verifiedPeerProcStart, msg_id, name, fromMode, body}`. Then `command_lifecycle started`, and at the end `command_lifecycle completed` after the model's answer, which obeyed the message (`DONE PEER`). | As E1. |

Sender side, all runs: stdout has `assistant tool_use SendMessage {to, message, …}` and a `tool_result` with `{"success":true,"message":"“<text>” → recv-31 (another Claude session on this machine; in that session's inbox, not yet read by its Claude …)","msg_id":"…"}`. **The sender's `msg_id` equals the receiver's `origin.msg_id`** (checked). `ListAgents` returns the sender's own name and ref and the peers with kind, status and age.

Facts E2 fixes: the receiver's identity claim (`verifiedPeerPid`, `verifiedPeerProcStart`) is **set by the CLI from the socket's peer credentials**, not from the message text; `origin` is a field on the frame, so text in a message body or a tool result cannot forge it.

### 1.3 What srv and the pane do with it today

- srv launches Claude as `-p --input-format stream-json --permission-prompt-tool stdio …` and does **not** pass `--replay-user-messages` (`agent_runtime.rs`; no match in `crates/srv`). So the peer message never reaches `stdout_reader.rs`, the transcript writer or the pane.
- The receiving pane shows a turn that begins with the model's reply (or tool calls) and no message above it. `command_lifecycle` is not read anywhere (no match in `crates/` or `frontend/`).
- The sending pane shows `SendMessage` as an ordinary tool row, with no recipient, no state, no link to the other pane.
- The jekt audit file (`<shared root>/audit/jekt-audit.jsonl`, written by `reactive/audit_store.rs`) and `GET /agentmux/reactive/audit` never see it, so `Message`, the Swarm/My Agents views and anything built on the audit show a conversation that did not happen.

### 1.4 Not tested here

Said once, so nothing below reads as verified when it is not: Claude **subagents and teammates** (`Agent` tool, agent teams); the **`[Cross-session delivery notice]`** and **idle notice** texts on the sender's side (the hold itself is verified, E3); **Codex**; any provider other than Claude. §9 lists who can close each.

## 2. Where it can be observed

| # | Where | What it gives | Live | Verified |
|---|---|---|---|---|
| O1 | Receiver stdout, a `user` frame with `origin.kind = "peer"` (needs `--replay-user-messages`) | from-name, sender pid (CLI-verified), mode, `msg_id`, body, the moment the CLI consumes it | yes | E2 |
| O2 | Receiver stdout, `command_lifecycle started / completed` with `command_uuid` = the frame's `uuid` | the message was read (started) and its turn ended (completed) | yes | E1, E2 |
| O3 | Sender stdout, `tool_use SendMessage` then its `tool_result` | to, text, `msg_id`, "accepted into the inbox" | yes | E1–E2 |
| O4 | Receiver's on-disk transcript, `queue-operation enqueue` | the message when it was **queued**, before the CLI read it (a busy or approval-held session) | tail the file | E1 |
| O5 | `<config dir>/sessions/<pid>.json` | session name ↔ pid ↔ cwd ↔ `status` | poll | §1.1 |
| O6 | srv's own process tree | CLI pid → block → agent (srv spawned it) | yes | srv tracks the CLI process it spawns per block (`pidregistry.rs`, `process_tree.rs`); the pid→block lookup is to be confirmed in P1 |

O1 + O3 are the record; O2 gives delivery state; O5 + O6 turn pids and CLI names into AgentMux agents; O4 is only for a message queued but not yet consumed (§6).

## 3. The record

### 3.1 Identity: who is "from" and "to"

The CLI names sessions by directory (`opaz-0909m-75`); AgentMux names agents by id. srv maps with what it already holds:

- **Receiver (this pane):** known, it is the block the frame arrived on.
- **Sender:** `origin.verifiedPeerPid` → srv's process tree → the block whose CLI process is that pid (or its ancestor) → the agent. Not found → "outside session", shown with the CLI name and pid, never with an agent name it could be guessed to be. `verifiedPeerProcStart` guards against pid reuse.
- **Sender, on the sender's pane (O3):** the pane's own agent; **recipient** is the `to` argument as the CLI session name. Resolving it to an agent through O5 (`name` → `pid` → block) is not built; the receiving pane's line (same `msg_id`) names the agent.

Names are never taken from the message text or from the CLI's `name` field: §3.5 is the full rule.

### 3.2 Fields

```
CliNativeMessage {
  msg_id:        string,   // the CLI's id: sender tool_result == receiver origin.msg_id
  from:          { agent?: string, cli_name: string, pid: number, mode: string },
  to:            { agent?: string, cli_name: string },
  sent_ms:       number,   // sender tool_use / tool_result time (O3)
  received_ms?:  number,   // receiver frame time (O1), else queue time (O4)
  read_ms?:      number,   // command_lifecycle started (O2)
  done_ms?:      number,   // command_lifecycle completed (O2)
  text:          string,   // the body, from origin.body (O1) or the tool_use message (O3)
  state:         sent | queued | read | done | held | refused | unknown,
  seen_by:       sender | receiver | both,   // which ends srv observed
}
```

States in jekt terms: `sent` = O3 success, "in the inbox"; `queued` = O4 only (not built); `read` = O1, the frame in which the receiving CLI hands the message to its model (the closest thing to "delivered" this channel has; in E2 its `command_lifecycle started` follows at once, so it adds nothing); `done` = O2 completed; `held` / `refused` = the delivery notice (§1.4, §9); `unknown` = only one end observed and nothing after. Never report `delivered` for `sent`: the CLI itself says "not yet read by its Claude".

### 3.3 One message, two observers

The sender's srv block and the receiver's block may be different panes of the same srv (the usual case) or of different srv instances (two AgentMux channels on one machine). The join key is `msg_id`. Each end writes its own lines (`cli_native.seen_by` = `sender` or `receiver`), within one srv and across instances alike, and the reader joins on `msg_id` (the audit line's `request_id`). Nothing is merged in memory: the two panes' readers do not need each other.

### 3.4 Where it is stored and how long

Same file, same writer, same route as jekts: a line in `audit/jekt-audit.jsonl` with `event_kind = "cli_native"` (the field already exists and defaults to `"delivery"`), `source_agent`, `target_agent`, `request_id = msg_id`, `outcome = state`, `message_hash`, `message_length`, plus new optional `cli_from_name`, `cli_to_name`, `cli_sender_pid`, `preview`, `text` (D2). The in-memory ring behind `GET /agentmux/reactive/audit` takes the same entries (cap 100 today; the file keeps all). Retention is the file's: never rotated, never truncated. A state change (sent → read → done) is a new line with the same `request_id`, as held jekts already do (`held_delivered`); the reader takes the last. That makes every existing consumer (the route, `worker_notes::absorb`, any "messaging audit" tool) see these messages once they learn the one `event_kind`.

The full text is also written to the agent's transcript record (§4.1), which is the history store: forever, searchable by `Agents search_history`.

### 3.5 Impersonation: what the CLI's peer identity proves, and what AgentMux shows

Under the operator's rule, forging who said something is the one thing to stop; everything else is allow-and-show. This section says exactly what can be trusted. Sources: the CLI's own schema text for the field (read from the pinned 2.1.288 binary), the socket file mode on charlie, and E2.

**What `verifiedPeerPid` is.** The CLI describes it as the "kernel-verified pid of the process that connected to this session's cross-session messaging socket, read from the connection (SO_PEERCRED / LOCAL_PEERPID), never from the payload". It names the **connecting process**, not the message's author: for relayed traffic (a daemon forwarding for another session) it is the relay. It is absent when unverifiable (Windows, non-socket ingress) and "never a wrong value". The CLI adds that pids are recyclable: "provenance, not an authentication token". `verifiedPeerProcStart` (the connecting process's start time) is the guard against reuse.

**What is claimed, not proven.** `origin.from` (the socket address; in E2 it read `/tmp/cc-socks/<pid>.sock` while the real socket is under `/run/user/1000/cc-socks/`), `origin.name`, `from-name`, `from-mode`, and the body are all written by the sender. The CLI's own text says `from` "is forgeable by any same-user process", and its own approval prompt shows the name only as "(peer claims name: …)" next to the verified pid.

**Who can connect.** The sockets are mode `srw-------` owned by the user, so **any process running as this user can connect** to any session's socket; the per-session token (`CLAUDE_CODE_MESSAGING_TOKEN`, also in a `0600` key file next to the registry entry) is in the environment of every command an agent runs. I did not establish whether the socket enforces the token, and this spec does not rely on it. A non-AgentMux process of this user, a script an agent wrote, or another agent's shell can therefore send a peer message. What none of them can do is choose the pid the kernel reports.

**Rule: AgentMux names the sender from its own pid mapping, never from the CLI's text.**

1. Take `verifiedPeerPid` (and `verifiedPeerProcStart`) from the frame's `origin`. Absent → "unverified sender", no agent name.
2. Walk that pid's parents in `/proc` until a pid srv recorded as the CLI process it spawned for a block (§3.1, `pidregistry.rs`). Found → that block's agent. Check the start time against `/proc/<pid>/stat`; a mismatch means a recycled pid → "unverified sender".
3. Not found (a process outside srv's trees: the operator's own terminal `claude`, a daemon that re-parented away from the pane, a script started elsewhere) → shown as "outside session (pid N)". **Never given an agent name**, whatever it claims.
4. The claimed name is shown only as a muted "claims to be `<name>`" when it differs from the mapped agent, and **never replaces** the mapped name. The body and `from-mode` are shown as sent, labelled as the sender's.
5. A relay (a pid that maps to a daemon, not a session) is shown as the relay.

**What this stops.** Agent B (or any process outside A's tree) cannot make a message look like it came from agent A: B's processes map to B, others map to "outside". A name typed into the message, or an `origin.name` of `"agenty"`, changes nothing the user sees except the muted "claims to be" line. A message body that contains the wrapper text or a fake jekt header stays plain text in the container (origin spec rule).

**What it does not stop, and the record says so.** A process **inside** agent A's own tree (A's shell, a script A runs) maps to A, so A can message through its shell as itself, and A's child cannot be told from A's CLI: that is A speaking, not someone else, and the audit line shows it as such. The `sender_pid` is stored so a reader can tell the CLI process from a child. A process that re-parents away from the pane is "outside", which fails safe. The record does not make the claim "authenticated" anywhere: the badge reads "same-user session, sender verified by pid", not "signed" and not "host-verified" (those words belong to jekts).

**Tests (P0).** The mapper is a pure function of (pid, start time, srv's pid table, a `/proc` view) and is tested for: the CLI pid maps to its agent; a child of it maps to the same agent; another agent's pid maps to that agent whatever name is claimed; a pid outside every tree maps to "outside"; a recycled pid (start time mismatch) maps to "unverified"; a claimed name equal to another agent's does not change the result; a missing pid maps to "unverified".

## 4. In the pane, live

### 4.1 Receiving pane

srv's stdout reader (`persistent/stdout_reader.rs`) already sees every frame. New rule, in one place: a `user` frame whose `origin.kind == "peer"` is **not** passed on as a user turn. srv writes one transcript record for it through the path the origin spec adds (its §3.2), with `amx.origin = {kind: "cli_native", from, trust: "cli-verified", cli_name, pid, mode, msg_id, sent_ms, delivered_ms}` and the body as content, signed like the others, and publishes the same typed value to the pane. The pane renders it in **the one container** (origin spec §4): header row "CLI-native · from `<agent or cli name>` · mode · time", body below, the trust badge reading "same-user session, verified by the CLI" (not a signature, not "host-verified": it is a different claim and says so). It appears at the moment the CLI reads the message, in order with the tool rows it lands between (E2), and the turn that follows is visibly caused by it.

Not the human's style, ever. A frame with no readable `origin` is never rendered as one of these and never as the human (origin spec §0, item 2).

The raw `<cross-session-message …>` wrapper and the CLI's trailing note ("This came from another Claude session — not typed by your user …") are not shown or parsed for display: the body comes from `origin.body`, the sender from `origin.name` and the verified pid. Nothing is classified from text (this is the origin spec's rule; a message body that contains the wrapper text must stay plain text).

### 4.2 Sending pane

srv recognises the tool call by its name (`SendMessage`, and `mcp__*__SendMessage` is **not** this tool: AgentMux's own `Message` tool is a jekt) and writes a record for it from the `tool_use` and its `tool_result` (`msg_id`, recipient, text, "accepted"). The pane shows it as a container row in the agent's turn: "CLI-native · to `<agent or cli name>` · sent", updated in place to "read" and "done" when the other end's lines arrive (same srv) or left at "sent" when it cannot know (§6). The generic tool row for `SendMessage` is replaced, not duplicated.

### 4.3 Other viewers

Mobile, history, Swarm and My Agents read the record and the audit line, so they show the same thing without learning the CLI's format. Search finds it: `Agents search_history` returns the text with `origin.kind = "cli_native"`.

## 5. Build plan

| Phase | What | Size / risk |
|---|---|---|
| **P0** | The sender mapper of §3.5 with its tests (nothing is shown or recorded under a name without it). Pass `--replay-user-messages` to Claude panes. Filter the stdout stream so the only replayed user frames that act are `origin.kind == "peer"`: the flag also replays the **human's typed prompts** (E2, line 3: `isReplay:true`), which srv and the pane already record themselves, so they must be dropped by `uuid`/absence of `origin`, not shown twice. Tests: a typed prompt appears once; a peer frame appears once; a resumed session's replay (`ClaudeCodeStreamParser({isReplay})`, `parseHistoryLines.ts`) does not re-record old peer messages (dedupe by `msg_id`). Check the flag's effect on the resume path and on the stream's size on a long session before enabling. | small; the risk is duplicates, so the test is the work |
| **P1** | `CliNativeMessage`, the sender/receiver join, the audit line and ring entry, the identity mapping (O5, O6), `command_lifecycle` states. Route and `search_history` return `cli_native`. | medium |
| **P2** | Transcript record with typed origin (rides the origin spec's P1) and the container rendering on both panes (rides its P2). | follows the origin spec; one more `kind` and one more header label |
| **P3** | Policy (D3), only if the operator wants more than visibility. | small to large |

P0 + P1 alone give the audit and the tooling (the second half of the request) without waiting for the container. They do not depend on the origin spec; P2 does.

## 6. What cannot be observed (and what the pane says then)

- **A message to a session srv does not host** (the operator's own `claude` in a terminal, a session on another machine, a cloud or Remote Control session): only the sender's half exists (`sent`, `seen_by: sender`). The CLI itself reports nothing back for Remote Control and cloud sessions, so the record says "sent, receipt unknown", never "delivered".
- **A message from a session srv does not host to one of ours:** the receiver's half exists; the sender is "outside session `<cli name>` (pid N)". The pid is verified; whose it is, is not known.
- **A message queued but not yet read** (receiver busy in a long tool call, or held for approval): the CLI has not emitted the frame. Only O4 (the transcript file) shows it, and only if srv tails that file. Without O4 the record stays `sent` until the frame arrives. Recommended: do not tail the file in P0–P2; show `sent`, and take O4 as a later improvement if the gap matters in practice.
- **Held (E3):** the recipient's Claude never sees it. The receiving pane's audit line says `held` with the CLI's cause, the sender `unverified` (the hold line carries no pid) and the text empty (it carries no body); the CLI's `message_uuid` is its `request_id`, not the sender's `msg_id`, so the two halves of a held message cannot be joined. The sender's pane still shows `sent`: its tool result said so, and the `[Cross-session delivery notice]` that should follow was not seen (§9). Agents whose permission modes differ cannot message each other through this channel.
- **Reads:** whether the receiving **model acted** on it is only visible as its turn that follows; `done` means its turn ended, not that it complied.
- **Subagent / teammate traffic** inside one CLI process (§1.4) is unverified; this spec makes no claim about it.
- **Version drift:** `peerProtocol: 1` and the `origin` shape are the CLI's, not ours. The reader treats an unknown `origin.kind` as "unknown CLI-native event" shown in the container with the raw kind, never dropped, and a contract test (the §8 harness, run against the pinned CLI in CI or by hand on each CLI bump) fails when the shape changes.
- **Other providers:** Codex, Gemini, Kimi, Antigravity, Qwen have no known equivalent here. See §9 for Codex.

## 7. Decisions

| # | Decision | Recommendation |
|---|---|---|
| D1 | Which end is authoritative for "delivered" | `read` (the receiver's `command_lifecycle started`) is the strongest claim the channel supports; `sent` is not delivery. Show both words. |
| D2 (**decided**) | Does the audit hold the full text? The jekt audit deliberately holds only a hash and length (`audit_store.rs`, "privacy, not security"); the operator's list for this record includes text. | Audit line: hash, length and a one-line **preview** (first line, ≤ 200 chars; the CLI shows the recipient's human the first line as well). Full text in the **transcript record** and `search_history` (already forever, already where jekt bodies live). If the operator wants the text in the audit file itself, say so; it is one field. |
| D3 (**decided**) | May agents keep using the native tool? It bypasses everything jekts have: signing, trust tiers, the sensitive tier's rules, queue/hold, quota hold. The CLI frames it to the receiver as "a teammate's request … likely working on their behalf". | **Allow and show**, now (operator: go with the recommendation). Impersonation is the one part that is stopped: §3.5. Revisit after a week of the audit: the record tells you whether anyone uses it. If the operator wants it off, `--disallowedTools SendMessage ListAgents` on every Claude pane is one flag, documented in the launch-args reference as the switch for it; the visibility work still covers sessions we do not launch. |
| D4 | Label | "CLI-native". Container header says it; trust badge says "same-user session, verified by the CLI". |
| D5 | The 6-sessions-per-identity reach: agents on one account see each other; the environment variables in every shell expose the socket and token to anything a pane runs. | Report, don't change here. A pane process that finds the socket can message agents without the CLI; that is only visible if it speaks the same protocol, and then O1 sees it. Whether to remove the variables from pane shells is a separate hardening decision. |

## 8. How to reproduce (evidence)

Isolated; nothing here reaches another agent. Needs the pinned CLI (`$CLAUDE_CODE_EXECPATH` in an agent shell).

```
X=/tmp/cli-native-xs; mkdir -p $X/cfg $X/recv $X/send
ln -s "$CLAUDE_CONFIG_DIR/.credentials.json" $X/cfg/.credentials.json
mkfifo $X/recv.in
setsid nohup sleep 3000 > $X/recv.in &            # keeps the fifo open; note its pid
# receiver (stays up, idle):
setsid nohup env -i HOME=$HOME PATH=$PATH CLAUDE_CONFIG_DIR=$X/cfg bash -c \
 "cd $X/recv && exec $CLAUDE_CODE_EXECPATH -p --input-format stream-json --output-format stream-json \
  --verbose --replay-user-messages --model claude-haiku-4-5-20251001 --permission-mode bypassPermissions \
  < $X/recv.in > $X/recv.out 2> $X/recv.err" &
printf '%s\n' '{"type":"user","message":{"role":"user","content":"Run three Bash calls: sleep 12; sleep 12; sleep 12. Then say DONE."}}' > $X/recv.in
# read the receiver's name from $X/cfg/sessions/*.json, then, from $X/send:
env -i HOME=$HOME PATH=$PATH CLAUDE_CONFIG_DIR=$X/cfg $CLAUDE_CODE_EXECPATH -p --output-format stream-json \
  --verbose --model claude-haiku-4-5-20251001 --permission-mode bypassPermissions \
  "Load SendMessage with ToolSearch (select:SendMessage), then SendMessage to <receiver name>: 'XS-TEST hello'. Then stop."
```

Look in `$X/recv.out` for the `origin` object and the two `command_lifecycle` lines, and in `$X/send.out` for the `msg_id`. Stop the sessions by pid (the `sleep` holder and the sessions listed in `$X/cfg/sessions`); remove `$X`. Each run costs a few cents on Haiku.

## 9. Open: to verify before the build

| Item | Who can close it | How |
|---|---|---|
| Subagents and teammates (`Agent` tool, `SendMessage` to "main" or a subagent): do their messages appear on the parent stream, with what shape | Opaz, with the §8 harness | one run each; add to §1.2 |
| The `[Cross-session delivery notice]` and idle-notice text and where they arrive on the **sender's** side (stream, transcript, or both). E3's sender run ended before any notice | Opaz | a sender that stays up after the send |
| Codex: whether its multi-agent feature (`multi_agent`, listed in `build_codex_side_question_argv`'s disabled set, so on in normal panes) produces items srv can see on the app-server or `codex exec --json` stream, and whether any of it crosses sessions or only sub-threads of one process | an agent that runs Codex (there is none on the Linux test machine) | list the item types of a run that uses it |
| Other providers | owners of each controller | same question |
| The flag's effect on resume and on stream size (P0) | Opaz | before enabling |

Nothing in §3–§5 depends on the Codex answer: it would be a second adapter into the same `CliNativeMessage`.
