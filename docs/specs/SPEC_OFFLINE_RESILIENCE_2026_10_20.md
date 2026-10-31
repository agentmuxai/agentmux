# SPEC: Stay in control of every agent when the internet drops, and recover by itself

**Date:** 2026-10-20
**Status:** active — §8 PRs shipped: A in #5213 (MuxBus: offline is not signed out), B in #5221 (notes ride with the next turn), C in #5215 (the pane shows an API retry), D in #5236 (offer the layout another build saved) and #5359 (closing is not deleting: a close keeps the window in the workspace as a closed window, `SPEC_WORKSPACES_2026_10_23.md` §5; it supersedes #5356); E's first part (§4.1 connectivity state, the status bar's Offline indicator) in #5241, its second (§4.4 held input) in #5246, E3 in #5353 (§4.3 recovery of failed turns), srv sending held input for a parked agent no pane shows (§4.4a). F and G remain.
**Related:** [`SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md`](SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md) (the per-class recovery banner),
[`SPEC_PERSISTENT_CONTROLLER_FAILURE_CLASSIFICATION_2026_08_04.md`](SPEC_PERSISTENT_CONTROLLER_FAILURE_CLASSIFICATION_2026_08_04.md),
[`SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md`](SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md).

## 1. Why

On 2026-10-20 the operator's internet went away for about 15 minutes (the router came back on the LAN after 40 s; nothing got past it). Two AgentMux instances with twelve agents were running. The operator lost control of all of them:

- Every agent looked busy and answered nothing. AgentMux had itself sent each one an `[AgentMux] Cloud messages are paused …` note as a user message, so every agent started a turn against the unreachable API and sat in the Claude CLI's 10-retry loop for about 7 minutes. Typing "u there?" started another 7-minute loop.
- When the retries ran out, AgentMux recorded the error as an unknown, non-retryable failure, so no recovery was offered.
- About two minutes of "cannot connect" made both instances decide MuxBus needed a fresh sign-in. Signing in needs the network; signing out deleted the stored sign-in, which would otherwise have recovered on its own.
- Nothing anywhere said "offline". Closing the windows was the only visible way out, and closing an instance's last window deleted its workspace: 39 panes in 5 tabs.
- After restarting, two forked agents came back under a different address, and MuxBus stopped delivering to them.

The network was outside AgentMux's control. Everything else in that list was AgentMux's doing.

## 2. Guarantees

These are the acceptance criteria. Each one is tested by the scenarios in §7.

| # | Guarantee |
|---|---|
| G1 | **You can always stop an agent.** Esc or Stop ends any turn within 2 s, including while the CLI is waiting between retries. |
| G2 | **You can always tell what is happening.** Within 10 s of losing the internet, the status bar says **Offline** (per service: the model API, MuxBus). A pane whose turn is retrying says so with the attempt, the delay and the cause, and offers Stop and Retry now. |
| G3 | **You can always talk to an agent.** Typing into a pane during an outage is never silently queued behind a dead turn: the message is shown as held, with "Send now" and "Cancel", and is sent by itself when the API is reachable again. |
| G4 | **AgentMux creates no load of its own while offline.** It never starts a model turn on its own initiative (notes, scheduled prompts, ambient calls) while the target API is unreachable. |
| G5 | **Nothing is destroyed by an outage.** No sign-in, credential, layout, queued message or conversation is lost because the network failed, or because of anything the UI offers during one. |
| G6 | **Recovery is automatic.** When the internet returns: MuxBus reconnects and re-subscribes every live agent without a sign-in; held messages are sent in order; a turn that failed only because of the network is retried (or offered, per §4.3); one "back online after N min" note reaches each agent with its next turn. |
| G7 | **Local keeps working.** With no internet at all: same-host agent-to-agent messages, the Terminal, Stop, panes, layout, and every local tool work exactly as before. |
| G8 | **Closing is not deleting.** Closing a window or quitting keeps its workspace; the next launch restores it. Only an explicit "Close workspace", confirmed, deletes one. |

## 3. Gaps against the guarantees

| # | Today |
|---|---|
| G1 | Stop sends the CLI `control_request {subtype: interrupt}`, so it should work mid-retry; not tested offline. |
| G2 | The CLI emits `{"type":"system","subtype":"api_retry","attempt","max_retries","retry_delay_ms","error_status","error"}` on stdout for every retry; AgentMux ignores it. The final `API Error: Connection refused … (ECONNREFUSED)` is classified `UnknownNonZero` (`agents/failure.rs` matches `econnreset` and `connection error`, not `econnrefused` or `connection refused`). There is no Offline state anywhere. |
| G3 | Input during an active turn joins `pending_send_messages` with no indication that the turn is stuck. |
| G4 | `muxbus/delivery_status` sends the pause note to every agent as a user message, starting a turn in each. |
| G5 | The broker's `NEEDS_REAUTH_THRESHOLD = 5` counts transport errors (no HTTP response) the same as rejected tokens, so ~2 min offline means "needs a fresh login". Sign out (`muxbus.disconnect`) deletes the stored credential, offline or not, and logs nothing. Closing an instance's last window runs `delete_workspace`. |
| G6 | After `NeedsReauth` the background sweep stops refreshing. After a sign-in, a subscription whose agent has no pane under that address is dropped, and an agent registered under a new address is not subscribed. |
| G7 | Believed true; not tested with the network down. |
| G8 | See G5: `window_closed` → `window_cleanup_cascade` → `delete_workspace`. `internals/persistence.md` documents the opposite. |

## 4. Design

### 4.1 One connectivity state (srv)

A `connectivity` module in srv holds the reachability of each service AgentMux depends on: each provider's model API host, MuxBus (relay and auth), and "internet" as a fallback.

- **Inputs:** outcomes reported by the subsystems that already make requests (cloud subscriber, broker, discovery, ambient calls), the CLI's `api_retry` frames and failure classes, OS network-change notifications, and a light active probe (TCP connect, then a HEAD) run only while a service is believed down, with capped backoff (5 s doubling to 60 s), and immediately on an OS network change.
- **Network changes (as built, `connectivity/netchange.rs`):** on Windows `NotifyIpInterfaceChange` reports a change; after 2 s with no further one, every service is probed again at once: one believed down starts its backoff over, one believed up is checked once (a failure there counts like any request, so one failed probe is `Degraded`, not `Offline`), and a probe already running is repeated when it ends. Linux (netlink) and macOS (`NWPathMonitor`) have no watcher yet; their probes run on their own schedule.
- **A pane's own API address (as built, `connectivity/endpoint.rs`):** a CLI started with a model API address other than srv's own (`ANTHROPIC_BASE_URL`, `OPENAI_BASE_URL`, `GOOGLE_GEMINI_BASE_URL`: a proxy or gateway) is its own service, `model_api:<vendor>@<host>[:port][/path]` ("Anthropic API (proxy.example.com)"; two routes of one gateway are two services, and a scheme other than https is part of the name), listed as `unknown` and probed at once when the CLI starts. Its frames and turns report on it, and the probe goes to that address, not srv's. The address is stripped of any user, password and query before it is an id, a label or a probe URL; one that can't be read as an address with a host is treated as no address and nothing of it is kept. At most 8 such addresses are tracked: past that the oldest one that is online or not checked yet is dropped. The pane learns its service from `agent:api_endpoint` in its block meta and holds input on that row. A pane on the default address is unchanged, and so is a container pane or one with a proxy variable srv doesn't have: their CLIs reach the API by a path srv's probe doesn't take, so they have no address of their own.
- **States per service:** `Online`, `Degraded` (some requests fail), `Offline` (every recent request fails at the transport level: DNS, connect, reset, TLS handshake, timeout). An HTTP response of any status means the service is reachable. A model API srv tracks is listed from startup as `Unknown` and probed at once, so a window can tell "not checked yet" from "answers" (#5246); held input waits on `Unknown` for at most 20 s.
- **Outputs:** an event `connectivity:changed` with the per-service states and `since`, an RPC to read it, and a MuxAPI field so agents can ask.

### 4.2 Pane: show the retry, offer Stop and Retry now

- srv translates `api_retry` frames into a block state `agent:retrying {attempt, max, next_at, cause}`, cleared by the next content frame or the turn's end.
- The working row shows: "Can't reach the Anthropic API (connection refused). Retrying, attempt 3 of 10, next in 18 s." with **Stop** and **Retry now** (Retry now interrupts and resends the same prompt).
- `agents/failure.rs` classifies `ECONNREFUSED`, "connection refused", `ENOTFOUND`, `ENETUNREACH`, `EHOSTUNREACH`, `ETIMEDOUT`, and the "Unable to connect to API (…)" text, as `Network` (retryable). It matches text only: Claude Code's `isNetworkDown` flag is internal to the CLI (checked in 2.1.288: its `api_retry` frame carries `error` and `error_status`, not the flag), so only the message it produces reaches AgentMux, and the turn's end is shown with the Network recovery: "The turn stopped because the API was unreachable. [Retry] (retries by itself when you're back online)".

### 4.3 Recovery of failed turns

When a turn ends with `Network` and connectivity for that provider later becomes `Online`:

- If the operator has not typed into that pane since, AgentMux retries the turn once by itself and says so in the pane.
- If they have, their held message (§4.4) goes first and the failed turn is offered, not retried.
- A setting turns automatic retry off.

### 4.4 Input while offline

- Input to a pane whose turn is retrying, or whose provider is `Offline`, is **held**: shown under the composer as "Held: will send when the API is reachable" with **Send now** and **Cancel**. Send now interrupts the retrying turn first.
- Held messages survive a pane reload and an app restart (they live with the pane's other persisted state).
- When the provider is back `Online`, held messages are sent in order.

#### 4.4a Held input for a parked agent

A held message goes out from the pane, so one for an agent no pane shows (a parked block, `agent:parked`: no tab, as after the pane of an agent that keeps running is closed) was never sent. srv sends it, from `server/held_input.rs`, every 20 s for each parked block that holds input:

- **When:** the block's agent runs and is between turns, and its model API is not `Offline` (and not "not checked yet" for under 20 s), the pane's own rule. Only the oldest message, one at a time. Plain messages and ones with attachments; a `/` or `!` command is never held.
- **Claim, so a pane mounting mid-send cannot send it too:** before a message goes, srv rewrites `agent:held_input` in one transaction that also checks the block is still parked, marking the message `unconfirmed` with an `attempt`, which is the mark a pane writes before it sends. A pane that attaches reads that as "may have gone" and leaves it to the operator; a pane attaching first clears the parked mark in a transaction of its own, so srv claims nothing. One of the two sends.
- **After:** srv's accept takes the message off the list (an empty list removes the key); a send that failed marks it held again and the block is left alone for 60 s. If srv stops between the mark and its accept, the message stays `unconfirmed` and waits for the operator, as it does for a pane.
- **Not done:** a stopped agent (the held message waits for a pane or a start), and a "N held" count on the Swarm row.

### 4.5 Notes never start turns

- `[AgentMux]` notes (pause, resume, overlap, any future kind) are queued per agent and delivered with that agent's next turn, as context, never as a turn of their own.
- Pause and resume notes for one outage collapse into a single note: "Cloud messages were paused 03:12–03:29 (no internet). Review and CI notices from that time may have been missed; check your PRs."
- Scheduled prompts (`Schedule`) whose agent's provider is `Offline` are held and fire once, on reconnect, not once per missed tick.

### 4.6 MuxBus: offline is not signed out

- **Broker:** a refresh that gets no HTTP response is `Offline`, not a failure: it does not count toward `NEEDS_REAUTH_THRESHOLD` and is retried with capped backoff indefinitely, and at once on `connectivity:changed → Online`. Only an answer from the auth server (`invalid_grant`, 400/401) can lead to needs-reauth.
- **States:** the delivery status distinguishes **Offline** (keeps the credential, reconnects by itself) from **Needs sign-in** (the server rejected it). The status bar shows which.
- **Sign out while offline:** confirm: "You're offline. Signing out deletes your sign-in, and you can't sign in again until you're back online." `muxbus.disconnect` logs at INFO.
- **Sign in while offline:** disabled, with the reason.
- **Reconnect:** after the WebSocket reconnects or a sign-in, subscribe and provision every live agent under every name it is registered by. A subscription whose name has no live pane is resolved against the agent's other names (uid, slug, former names) before it is dropped.

### 4.7 Durable layout

- Closing a window, or the last window, or quitting keeps the workspace; the next launch restores it. An explicit **Close workspace** (with confirmation, naming the running agents it stops) is the only way to delete one.
- Closing a window that has running agents warns: "This stops N agents. Their conversations are kept." with **Quit** and **Cancel**.
- On the first launch of a new channel, offer to bring over the previous channel's layout (the most recent channel this user ran), and its MuxBus sign-in.

**As built (D, closing is not deleting, #5359).** What each way of ending a window did before D:

| Path | Before D | Next launch |
|---|---|---|
| Close a window, others still open | `CloseWindow` closed its agents and ran `delete_workspace`; nothing was saved | that window's layout was gone |
| Close the last window | the same, after `save_on_last_close` wrote the workspace to `Client.meta["session:last_topology"]` | rebuilt the record into a new workspace (new pane ids) |
| Quit (tray, upgrade restart: the host's `quit_app`) | `SaveSessionSnapshot(for_quit)` saved every window, then each `CloseWindow` deleted its workspace | rebuilt every window from the record |
| OS shutdown or logoff | `SaveSessionSnapshot` only, no `CloseWindow`: every row survives | opens the surviving windows, as after a crash |
| Crash; Alt+F4 on the main window (no `CloseWindow` is sent) | nothing runs; every row survives | main reopens onto its workspace; secondary windows are reprojected onto fresh workspaces and the host then closes their old ids, which deleted the old workspaces |

☰ → Exit closes this window; it is not a quit.

Now (#5359, which supersedes #5356): a close keeps the window in the open workspace as a closed window, its agents stopped and its panes kept with the same ids; the version panel lists it greyed, to reopen or remove. How it is built, the close warning's wording and what #5356 had that didn't ship (☰ → Reopen closed window, Close workspace…, the cap of 10) are in `SPEC_WORKSPACES_2026_10_23.md` §5 "As built".

### 4.8 Stable agent address

- Migrate pre-fix fork slugs (`loap-loap-2-loap-3`) to `fork_slug`'s rule (`loap-3`), moving what is keyed by the old slug (the `gh-<slug>` config folder, name-keyed signing keys, registry entries) and recording the old slug as a former name, so messages to it still resolve.
- MuxBus subscriptions are keyed by the agent's uid, with names as aliases, so a rename or slug change cannot orphan one.

## 5. Out of scope

- Making the model API reachable offline, or running a local model as a fallback.
- Fixing the user's network.
- Behaviour of other agent CLIs' own retry loops beyond reading their events (§4.2 is Claude-first; other CLIs follow when they emit similar events).

## 6. Risks

- **Automatic retry repeats a side effect.** A turn interrupted mid-tool-call and retried may run the tool again. Retry only turns that failed before any tool ran (the CLI's `api_retry`/`api_error` before the first tool frame); otherwise offer, don't retry.
- **False Offline.** A captive portal or a blocked single host must not mark everything offline: states are per service, and an HTTP response of any kind means reachable.
- **Held messages surprising the operator** when they are sent minutes later: show them as held, with the time, and say "sent (held N min)" when they go.

## 7. Testing

### 7.1 Fault injection

A dev-only fault proxy (a small local HTTPS CONNECT proxy in `scripts/`) that the dev instance and its CLIs use through `HTTPS_PROXY`, with modes switchable at run time:

| Mode | Behaves like |
|---|---|
| `refuse` | TCP RST on connect (`ECONNREFUSED`): what happened on 2026-10-20 |
| `blackhole` | connect never completes (timeouts) |
| `dns` | name resolution fails |
| `flap:<on>/<off>` | alternates, e.g. `flap:30s/20s` |
| `pass` | normal |

Per-host rules allow "API down, MuxBus up" and the reverse.

### 7.2 Scenarios

Each runs in an isolated `task dev` instance with three agents mid-conversation; each guarantee's check is listed.

| Scenario | Checks |
|---|---|
| `refuse` for 15 min, then `pass` | G1 Stop < 2 s mid-retry; G2 Offline in status bar < 10 s, retry row with attempt/delay; G3 typed message held, Send now works; G4 no turn started by AgentMux; G5 MuxBus not "needs sign-in", credential kept; G6 on `pass`: MuxBus Connected without sign-in, held messages sent, failed turns retried once, one coalesced note |
| `blackhole` 3 min | same, with timeouts |
| `flap:30s/20s` for 10 min | no needs-reauth; no duplicate turns; state settles on Online at the end |
| API `refuse`, MuxBus `pass` | MuxBus stays Connected; only the API shows Offline |
| Sign out while `refuse` | confirmation shown; Sign in disabled with reason |
| Close last window during `refuse`, relaunch | G8: layout and held messages restored; agents resumable |
| No network at all (adapter disabled) | G7: same-host Message send, Terminal, Stop, panes work |

### 7.3 Unit tests

Broker: five transport errors stay `Offline`; an `invalid_grant` goes to needs-reauth. Classifier: the exact `API Error: Connection refused … (ECONNREFUSED)` string, as stderr and as a result frame, and the `Unable to connect to API (…)` strings are `Network` (`agents/failure.rs`). Notes: a pause note never starts a turn; pause+resume collapse to one. Translator: `api_retry` frames set and clear `agent:retrying`. Window close keeps the workspace. Slug migration moves the `gh-` folder and keys.

## 8. Plan

Ordered by how much control each returns; A, B and C are independent and can go in parallel.

| PR | Scope | Guarantees |
|---|---|---|
| A | Broker: transport errors are `Offline`, never needs-reauth; Offline vs Needs sign-in in delivery status; guarded Sign out and Sign in; log `muxbus.disconnect` | G5, G6 (MuxBus) |
| B | Notes never start turns; queue, coalesce, deliver with the next turn; hold scheduled prompts while offline | G4 |
| C | `api_retry` → `agent:retrying` and the retry row with Stop / Retry now; Network classification; Network turn-end banner | G1, G2 (pane) |
| D | Close keeps the workspace, restores on launch; close warning; persistence docs. Shipped: the other-build offer (#5236), and closing is not deleting (#5359, §4.7 As built) | G8, G5 (layout) |
| E | `connectivity` module; status-bar Offline per service (#5241); held input (§4.4, #5246) | G2, G3 |
| E3 | Recovery of failed turns (§4.3): automatic retry on reconnect, the offer when the operator typed since. See the notes below. Shipped in #5353 | G6 |
| F | MuxBus resubscribe and provision every live agent and alias after reconnect; uid-keyed subscriptions; fork slug migration | G6, §4.8 |
| G | Fault proxy and the §7.2 scenarios as a scripted run | all |

### 8.1 Notes for E3

#5246 first carried §4.3 too, and review found these; E3 starts from them.

- **Carry the attachments.** The retry re-sends the failed turn's message, and must send its images with it, not just its text.
- **At most once, single-flight.** An offered Retry, and the automatic one, must not go twice: a second click, or a click while the automatic retry is in flight, sends nothing.
- **An unknown outcome keeps a way to recover.** When srv's answer to the retry never comes (or the pane's send gives up waiting), keep the turn marked as possibly sent and leave the operator a Retry; don't mark it retried, and don't lose the failure row.
- **A failure read back is only offered.** One read back on mount (`agent:last_failure`, or the broker's replay after a reload) may have been dismissed before: never retry it automatically. Only a failure seen live in the pane is.
- **Dismiss means done.** Dismissing the failure row drops the turn, including one the operator typed after.
- **Network failures and the timed retry.** Skipping the failure row's timed retry for a tracked provider only makes sense together with the retry on reconnect; ship them together, and decide tracking before the pane has a status.

## 9. Open questions

1. Automatic retry of a network-failed turn (§4.3): on by default? Proposed: on, only for turns that failed before any tool ran. **Decided (E3):** on by default, only for a turn the operator started from that pane while it was open, that failed before any tool ran and isn't itself a failed automatic retry; any other is offered. `agent:retryonreconnect` (Settings › Advanced) turns it off.
2. Where held messages are stored: with the pane's persisted state (proposed) or the agent's history store. **Decided (#5246):** the block's meta (`agent:held_input`), sent by the pane once it is open and the API answers.
3. Whether the first launch of a new channel should copy the previous channel's layout automatically or ask (proposed: ask once).
