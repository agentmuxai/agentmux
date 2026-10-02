# SPEC: jekt delivery that tells the truth — receiver state, one mailbox, receipts

**Author:** AgentY (narko), at operator request
**Created:** 2026-10-01
**Status:** active — Phase 0 item 1 (pull on subscribe, connect and a timer) shipped in PR #4122 (v0.59.3); item 3 (`SendMessage` reports the state, the receiver's condition where known, and the id) is built (MCP side only, see §6); everything else is proposed. Designs what `SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md` §3 recorded as "Phase 2" and fixes the gaps found in the 2026-10-01 incident (§1).
**Related (read these first):**
- `SPEC_JEKT_IMMEDIATE_DELIVERY_2026_09_28.md` — current policy: a live agent gets the jekt at once; only a starting, restarting or stopping process queues. Implemented.
- `SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md` — the 24 h hold (`db_jekt_held`) for an absent agent. Phase 1 shipped (#3632); its Phase 2 is this spec.
- `SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md` — the relay lease, so one instance at a time owns an agent.
- `SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` — trust fields a stored jekt must keep (§8).
- `docs/retro/RETRO_DEFERRED_RESTART_NEVER_RESPAWNS_2026_09_29.md` — four jekts sat "QUEUED… starting up" for an hour.
- `SPEC_LAN_FIREWALL_SETUP_2026_10_01.md` (AgentX, proposed) — the same afternoon, LAN jekts between two machines failed with no warning anywhere because a Windows Firewall rule covered only the Public profile. The LAN tier fails silently in the same way the relay tier does (§3.4).
- Superseded, kept for history: `SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md`, `SPEC_JEKT_DEFERRED_DELIVERY_NO_MIDTURN_INTERRUPT_2026_09_10.md`.

## 1. The incident that prompted this (2026-10-01, UTC)

Lark asked AgentY to review a PR. AgentY's pane was not loaded. The operator opened it, hit a credentials
error, signed in, and asked AgentY whether it had seen Lark's message. AgentY said no. The message
reached the model four minutes after it was sent. Evidence from this instance's srv log
(`agentmuxsrv-v0.59.1.log.2026-10-01`):

| UTC | Event | Meaning |
|---|---|---|
| 03:06:26.795 | message id `inj-1790823986795-…` created by Lark | sent once, no retry |
| 03:06:26.901 | `reactive inject request received` target `agenty` | srv takes the request; AgentY has no registered process |
| — | (no log line) | Lark's tool returned: *"QUEUED for agenty via the cloud relay — NOT yet delivered … You get this same result for an agent name that does not exist anywhere"* |
| 03:07:04 | pane opened; `identity.spawn.blocked: no credentials for provider claude`; eager-resume declined, registered lazily | the agent exists and is **credential-blocked**; nobody is told |
| 03:07:11 | sign-in done; process spawned; agent registered | AgentY is now live |
| 03:07:17–03:07:57 | operator asks "u there", "did u see the message from lark?" ×2 | the model has not received it |
| 03:07:19 | `muxbus: provisioned per-agent credential` for `agenty` | relay credential in place; the subscription follows |
| 03:10:25.489 | `wan verify: arrived without a WAN signature` for the same id | pulled from the relay **3 min 59 s after it was sent, 3 min 6 s after AgentY went live** |
| 03:10:25.493 | `inject: structured delivery … target agenty` | first moment the model could see it |

> **Update 2026-10-01 (after merge):** the pull gap described next was fixed independently by AgentX in PR #4122 (merged 00:59 PT, shipped in v0.59.3), which also records a second occurrence: a jekt waited 8 min 46 s on 2026-09-30. See G1 and Phase 0. The text below describes the code as it was when the incident happened.

Why it was late: `cloud_subscriber.rs` pulls pending relay messages **only** when the relay broadcasts
`inject_available` (`sync_agent_reactive` is called from that one handler; its own comment says "no
periodic resync exists"). Subscribing a new agent (`SubscribeAdd`) or reconnecting does not pull. The pull
at 03:10:25 coincided with an unrelated delivery to Lark in the same second, consistent with another
injection's broadcast waking it (I did not capture the relay side to confirm). Had nothing else been sent, the message would have sat until the relay's
30-minute expiry and then vanished, and **no one would have been told**.

Also, this was a same-machine message: Lark and AgentY run in the same srv. It went to the cloud and
back because the cloud relay is tried before the local hold (`server/reactive.rs:1071` vs `:1077`).

**Not explained (open, §10 O1):** the operator reports the message "appeared when I opened your pane".
The code renders a jekt in the transcript only when it is written to a process (`append_delivered_message`),
and the log shows no such write before 03:10:25. What the operator saw needs a screenshot or the pane
transcript; do not design around the guess.

## 2. How we got here — the twists

Each step fixed a real loss and each left a place where the sender is told something untrue. Dates and
PRs are from `git log` and the specs above.

| Date | PR / doc | Change | What it left open |
|---|---|---|---|
| 2026-03-13 | #117 | Jekt: PTY keystrokes, auto-registration, cross-instance routing | PTY only |
| 2026-06-15 | delivery-hierarchy spec | Tiers 1–4; §10 "in-memory is fine, lost on restart, users accept this" | tier 4 was a comment |
| 2026-06-16 | #1477 | Controller-aware delivery: structured agents get it on live stdin | — |
| 2026-07-10 | #2077 | Incoming jekts visible on persistent agents | — |
| 2026-07-29 | #2338 | Fast-fail when the **human** composer sends while unauthenticated | no jekt-side equivalent (§3 G3) |
| 2026-09-02/03 | #2930, #2960 | Subprocess and unspawned persistent agents were silently dropped; now they receive it (first jekt spawns them) | slow spawn can look like a transport timeout |
| 2026-09-06 | #3022/#3023 | Cloud relay becomes tier 4 (before this "local → LAN → cloud" stopped at LAN) | relay accepts any name |
| 2026-09-11 | #3188 | `SendMessage` says Delivered vs QUEUED (before: "Message sent" for a nonexistent name) | QUEUED never resolves for the sender |
| 2026-09-17 | cloud #76 | relay abandons undelivered jekts after 30 min | sender is never told |
| 2026-09-23 | #3557/#3562 | hold automated messages to a turn boundary; in-memory `deferred_deliveries` | lost on stop or crash |
| 2026-09-24 | #3632 | durable 24 h hold, `db_jekt_held`, reported as `HELD` | Phase 2 never started |
| 2026-09-24/25 | #3738…, cloud #92 | single live instance per agent (relay lease, fencing) | — |
| 2026-09-27/28 | #3979, #3977 | **reversal:** live agent gets the jekt at once, no turn-boundary hold | durable hold unchanged |
| 2026-09-29 | #3990 | deferred restart now respawns (the Korp outage) | "queued forever" still invisible to senders |
| 2026-10-01 | #4122 (AgentX) | catch-up pull on connect, on add, and every 120 s; subscription seeded with agents that registered before the subscriber existed | sender still told only "QUEUED"; relay expiry still silent |
| 2026-09-29/30 | issues #4012, #4013 | MuxBus sign-in expires after ~2 h → sends fail "agent not found"; late-login agents not subscribed | both open |
| open | issue #3894 | review notices arrive 1–1.5 h late: held/deferred jekts ignore the sender's expiry | open |

**The pattern.** Three times the answer to "a message was lost" was "add a place to keep it" (deferred queue,
durable hold, relay). Each place got its own sender string and its own expiry, and none of them reports the
outcome back. "Delivered" has quietly come to mean "handed to the next hop", and "QUEUED" means "we hope".
The 2026-10-01 message crossed three of these places and the sender could not tell which.

## 3. Today's behaviour and its gaps

Measured on `origin/main` (`c91c140b4`).

### 3.1 What the sender is told

`crates/mcp/src/tools/fleet.rs`: `Delivered to X — injected…` (:85) · `QUEUED for X — their agent is starting up…`
(:83, `tool_helpers.rs:187`) · `QUEUED for X via the cloud relay — NOT yet delivered…` (:89) ·
`HELD for X — … up to 24 hours` (:102) · `Message delivery failed: …` (:111). The tool never reads the
`request_id`, so the sender is not given a message id. `tool_schemas.rs:149` documents none of `HELD`
or the failures.

### 3.2 Receiver states and what really happens

| Receiver state | Path | Sender told | What actually happens | Loss risk |
|---|---|---|---|---|
| live, idle or busy | stdin write | Delivered | written at once; the CLI reads it at its next step | none if the CLI is healthy |
| live, **CLI unauthenticated** | stdin write | Delivered | rendered in the pane; CLI returns an auth error; model never acts; retry-after-login resends only `user_message` nodes, not `jekt_message` | **silent** (G3) |
| starting / restarting / stopping | in-memory `deferred_deliveries` (cap 64) | QUEUED "don't resend" | flushed by a 500 ms watchdog; with no process and no spawn for ~10 s the queue is dropped with a log line only | **silent** (G5) |
| defined, no process (pane not loaded or stopped) | relay **first**, local hold second | QUEUED via relay (signed-in sender) or HELD | relay: pulled only on a wake (§1), expires at 30 min; hold: replayed on registration or every 30 s, dropped after 20 failed attempts or 24 h | **silent** (G1, G2, G6) |
| defined, **spawn gate refuses** (no credentials, deleted account) | none | `Message delivery failed: identity spawn gate…` | not held, not queued; for host/LAN senders the jekt is dropped; for WAN it is released back to pending | the sender is told it failed, but not that retrying later would work (G4) |
| other machine, LAN | mDNS lookup, then forward | Delivered or QUEUED per the peer | a peer that never resolved (firewall, mDNS) is simply not found, so the jekt falls to the next tier (relay, then hold) and the sender sees QUEUED via the relay, with nothing saying the LAN tier is down | **silent** (§3.4) |
| other machine, WAN | relay | QUEUED per the relay | WAN `delivered` on the relay = the receiver's srv claimed it, not that the model has it | **silent** on expiry (G9) |

### 3.3 Gaps

- **G1. The relay path is wake-driven. FIXED in #4122 (v0.59.3).** Before: no pull on `SubscribeAdd` or connect, no periodic resync (`cloud_subscriber.rs`, `sync_agent_reactive` doc), so a message to an agent that came online *after* the send waited for an unrelated wake. This was the incident. Now `catch_up` runs on connect, when an agent is added, and every `CATCH_UP_EVERY` = 120 s, and registered agents are seeded into the subscription when the subscriber is created (an isolated channel creates it lazily at `muxbus.login`). What remains of G1: a missed wake now costs up to 2 minutes, not an unbounded wait; and the relay's 30-minute expiry still drops the message silently (G9).
- **G2. Relay before local hold, even for a same-machine target.** A defined local agent gets a cloud round trip it does not need, and the 30-minute relay expiry instead of the local 24 h hold.
- **G3. "Delivered" means enqueued, not consumed.** srv has no per-agent auth state on the jekt path; the human fast-fail (`useAgentCommands.ts`) is not reachable from `/agentmux/reactive/inject`.
- **G4. Spawn-gate refusal drops, with no hint.** `identity spawn gate: …` is a recoverable condition (sign in) reported like a permanent failure, and the jekt is not kept.
- **G5. Stranded deferred jekts vanish** after the sender was told "don't resend" (`input.rs:430-454`, 488-501; the code's own comment calls it a gap).
- **G6. A held row is dropped after 20 non-transient failures** and the sender is not told (`server/jekt_held.rs:19,181`). A spawn-gate refusal counts as one of the 20.
- **G7. There is no mailbox to read.** No MCP tool or RPC lists held, deferred or relay-pending messages. An agent coming online cannot ask "what did I miss?".
- **G8. Presence is only "registered".** `DiscoverAgents.addressable` means "in the handler's name map" (`handler.rs:616-623`); a CLI that is up but unauthenticated is `addressable: true`. No auth state, no queue depth.
- **G9. The sender never learns the end of the story.** The relay's `status` endpoint exists (`GET /reactive/status/:id`) but nothing calls it; `expired` is declared and never set; after 30 min the row is simply filtered out.
- **G11. A broken LAN tier is invisible to the sender.** `SPEC_LAN_FIREWALL_SETUP_2026_10_01.md` records two machines on one LAN where each saw a different peer list and no LAN jekt could be sent, with no warning in either log. A send to a LAN peer that never resolved looks the same as a send to a name that does not exist.
- **G10. `SendMessage`'s description is stale** (omits `HELD` and the errors, still says an unknown name "also returns QUEUED" as the only caveat).

### 3.4 Silent failure is the common thread

The relay (G1), the deferred queue (G5), the held row (G6), the spawn gate (G4) and now the LAN tier (G11) each lose or strand a message without telling the sender. They were found one at a time, over six months, each after a person noticed a missing message. The design below treats "every path reports its end state" as the requirement rather than fixing the paths individually.

## 4. Principles

1. **State, not success.** The sender is told the jekt's *state* and the receiver's *condition*, never a bare success.
2. **One mailbox.** Every jekt that is not written to a ready process lives in one durable store with one set of rules, whichever tier accepted it.
3. **Local first, but only where this srv owns the agent.** If the target is an agent this srv knows **and this srv holds its single-live-instance lease, or nobody does**, the mailbox is here. If another instance holds the lease the agent is live there, and a local hold would strand the message: it goes to the relay, which that instance pulls from (§5.3 *Leases*). The relay is for agents on other instances.
4. **Online means drain.** When an agent becomes able to take a message (spawn succeeded, signed in, restarted), its mailbox drains. Nothing waits for an unrelated event.
5. **Every end state is reported.** Delivered late, expired, failed: the sender hears, once, in the same channel (a jekt).
6. **Never claim more than we know.** `delivered` = written to a process srv believes is authenticated. `read` is a separate, optional state (§5.4).

## 5. Design

### 5.1 The states a jekt moves through

```
accepted ─┬─> delivered ──> read (optional)
          ├─> held(reason) ──> delivered | expired | failed
          └─> relayed ──────> delivered | expired | failed
```

`reason` for `held`: `not_running` · `pane_not_open` · `needs_login` · `starting` · `restarting`.
`relayed` is for agents known to be on another instance. A jekt never goes straight to `failed` while
retrying could still succeed (that is `held`).

### 5.2 What the sender is told

Keep today's first word (`Delivered`, `HELD`, `QUEUED`), so existing agent prompts keep working, and add
the receiver's condition and the message id. Examples:

```
Delivered to agenty (active) — written to their running conversation. id=inj-…
Delivered to agenty (idle) — …
HELD for agenty (needs_login) — their pane is open but not signed in. The message is kept for up to 24 h and delivered the moment they sign in. You will get a jekt if it expires. Do not resend. id=inj-…
HELD for agenty (pane_not_open) — … delivered when they open the agent. …
QUEUED for agenty via the cloud relay (unconfirmed, expires in 30 min) — an agent on another AgentMux instance. …
Message delivery failed (not_found): no agent named "agentx" is known here, on the LAN, or on the relay…
```

`/agentmux/reactive/inject` returns the same facts as fields: `state`, `receiver_state`, `msg_id`,
`expires_at` (additive; old clients ignore them). The relay-before-hold order is reversed for local agents
(G2), so a same-machine target is `HELD` here and never `QUEUED via the cloud relay`.

`receiver_state` is `active` (live, authenticated, in a turn), `idle`, `starting`, `needs_login`,
`stopped`, `pane_not_open`, `unknown` (remote or unresolvable). srv derives it from the controller
(status, spawn gate) and the CLI auth classification that already exists at spawn
(`spawn.rs:1270-1281`); a controller whose CLI reported an auth failure stays `needs_login` until a turn
succeeds or the user signs in.

### 5.3 The mailbox

Generalise `db_jekt_held` (migration v40) into the mailbox; do not add a second table.

- Rows are keyed by `msg_id`; add `state`, `reason`, `delivered_at_ms`, `read_at_ms`, `notified_sender`.
- Everything not written to a ready process goes in: the in-memory `deferred_deliveries` queue (G5), spawn-gate refusals (G4), jekts for a `needs_login` controller (G3), and local-agent messages that today go to the relay (G2).
- Retention: undelivered 24 h (unchanged); delivered/expired/failed rows kept 7 days for `MessageStatus` and receipts, with the body cleared on delivery (only trust metadata and a length stay).
- **Leases.** A mailbox row belongs to the instance that holds the agent's lease. On Take over or fencing (`not_holder`), the instance that lost the lease re-posts its pending rows for that agent to the relay under the **same `msg_id`** (the relay's idempotency key) and marks them `relayed`; the instance that gained it pulls them like any relayed message. A row whose re-post fails keeps `state = held` and gets `repost_pending = 1` (a new nullable column, with the other mailbox columns above). The existing 30 s replay pass already walks every row (`server/jekt_held.rs`); it gains one rule: a row with `repost_pending` is re-posted to the relay first, and is delivered locally only if this srv holds the agent's lease again, in which case the flag is cleared. The sweep therefore retries exactly the rows a Take over left behind, and a row is only ever in one place: local while this srv holds the lease, on the relay once it has been re-posted. Because the relay dedupes on `msg_id`, a re-post that succeeded but whose reply was lost is harmless to repeat. A send that arrives while the lease is held elsewhere is relayed, never held locally.
- Caps unchanged (64 per target, 1000 per channel); a full mailbox is a `failed(mailbox_full)` the sender is told about, never a silent drop.
- A spawn-gate refusal is **not** an attempt against the 20-attempt budget; it is `needs_login` and waits for the gate to pass. The attempt budget applies only to genuine delivery errors, and exhausting it is a `failed` the sender hears about (G6).
- **Drain triggers** (all idempotent, ordered by send time): agent registered; spawn succeeded; sign-in completed (the `CheckCliAuth` / in-app login success path); restart finished; a 30 s sweep as the backstop that already exists. This replaces "waits for an unrelated wake" (G1).
- **Relay side (agentmux-cloud and srv).** On `SubscribeAdd` and on connect, srv runs `sync_agent_reactive` for the added agents immediately, and a 60 s periodic resync backs it up (cheap: one `GET /reactive/pending/:id` per subscribed agent). Relay messages set `expired` when they expire, and srv, as the sender's instance, tells the sender (§5.5).

### 5.4 What the receiver gets, and can ask for

- **On drain**, the agent receives its missed messages in order, each with its original JEKT marker, whose `TS=` is the original send time, so the delay is visible (a replayed held row already carries `held_sent_at_ms`, which the pane uses to show send time and wait). If more than 3 are waiting they arrive as one **digest** jekt (`TIER=info`, summarising count, senders, ages, then the messages), so a returning agent isn't flooded turn by turn.
- **`InboxList`** (new MCP tool): the agent's own pending, held, and recently delivered messages: id, sender, age, state, first 120 characters. Read-only; an agent can only see its own mailbox. This is what "check next time they come online" needs, and it is the first thing a returning agent can do when it suspects it missed something.
- **`MessageStatus(msg_id)`** (new MCP tool, sender or receiver): state, reason, times. Backed by the local mailbox, falling back to `GET /reactive/status/:id` for relayed messages (which no client calls today, G9).
- **`read`** (optional): the CLI's user-message echo for that turn marks the row read. Not required for any behaviour in this spec; it exists so "delivered" never has to be stretched to mean "understood".
- **Operator view:** the agent pane shows "N messages waiting" (the Phase-2 UI the durable spec recorded) with list and cancel. Cancelling notifies the sender.

### 5.5 Telling the sender the end of the story

When a message the sender was told was `HELD` or `QUEUED` reaches an end state, the sender gets one system jekt (`TIER=info`, from `system`, never itself receipt-eligible, so no loops):

- **delivered late**, if it waited more than 60 s: "Your message to agenty (id …) was delivered at 03:10 after 4 min (held: needs_login)."
- **expired**: "Your message to agenty (id …) expired undelivered after 24 h (reason: needs_login). Their pane has been waiting for a sign-in."
- **failed / cancelled**, with the reason.

Rate-limited per target (one summary per minute, merging several). The sender's instance is the one that
holds the row (local mailbox) or the relay record (relayed); an instance that is offline gets the
notice on its next sync, like any relay message. This is what makes "Do not resend" safe to say.

### 5.6 Presence

Where a LAN peer was expected but not resolved, the result says so (`QUEUED via the cloud relay (LAN peer not found — LAN may be unavailable here)`). The indicator and reasons from `SPEC_LAN_FIREWALL_SETUP_2026_10_01.md` (§ on the status-bar LAN state) are the source for "LAN unavailable"; this spec only asks that the send result use them rather than duplicate the check.

`DiscoverAgents` adds, per agent: `state` (§5.2), `mailbox_pending`, and `last_turn_at`. `addressable` stays
(back-compat) but is documented as "registered". A sender that cares can check `needs_login` before sending;
a sender that doesn't is told it in the send result (§5.2).

## 6. Phases

- **Phase 0 — truthful and unstuck (small, no schema change).**
  1. ~~Pull on `SubscribeAdd`/connect and a resync in `cloud_subscriber.rs` (G1).~~ **Done: PR #4122** (v0.59.3), with a 120 s timer rather than the 60 s proposed here. Its unit tests cover the subscription seeding; the end-to-end timing check is test 2 below and is still to be written.
  2. Local-first, **lease-aware** (G2): a known local agent is held locally only if this srv holds its lease or the relay reports it free. The relay today exposes only `claim`, `take`, `renew` and `release` for leases, so this needs a **read-only holder query** (`GET /agents/lease/:agent`, same auth as `/reactive/pending`). Until it exists, a target whose lease state srv does not know keeps going to the relay as today; 0.1 already makes that path deliver on subscribe, so the §1 scenario is fixed without 0.2.
  3. ~~`SendMessage` returns `id=` and the receiver condition where srv already knows it; refresh `tool_schemas.rs` (G10).~~ **Built (2026-10-02):** `send_message_outcome` in `crates/mcp/src/tool_helpers.rs`, MCP only, since srv's inject body already carried every fact needed. Each answer keeps its first word and ends `id=<request_id>`; conditions added: `HELD … (not_running)`, relay `(unconfirmed, expires in 30 min)`, `failed (needs_login)` for a spawn-gate refusal (says the message was not kept, until 0.4), `failed (not_found)`. Not added: `active`/`idle` on `Delivered` (srv does not know the turn state on this path) and which of starting/restarting/stopping deferred a message; both need the `receiver_state` plumbing of Phase 1.
  4. A spawn-gate refusal is reported as `HELD (needs_login)` and the jekt is kept (G4).
  Acceptance: the §1 scenario passes (§7 test 1). With 0.1 shipped, the pull half already passes; what still fails is the sender-visible half (a same-machine target is still answered "QUEUED via the cloud relay" rather than HELD, which is 0.2; 0.3 now adds the id and the relay's expiry to that answer) and the credential-blocked half (0.4).
- **Phase 1 — durable and drained.** Mailbox columns and states (§5.3); move `deferred_deliveries` into it (G5); `needs_login` hold with drain on sign-in (G3); attempt budget fix (G6); `receiver_state` plumbing.
- **Phase 2 — receipts and the mailbox tools.** Sender notices (§5.5), `InboxList`, `MessageStatus`, digest (§5.4), `DiscoverAgents` presence (§5.6), relay `expired` status.
- **Phase 3 — UI.** "N waiting" badge, list, cancel.

Phase 0 is independent of the rest. Its first item removed the unbounded wait; the remaining items (0.2 and 0.4; 0.3 is built) are what make the sender's picture true, so they are the next thing to build.

## 7. Tests

1. **The incident, end to end:** pane not loaded → send → sender sees `HELD (pane_not_open)` and an id → open the pane with the account unresolvable → state becomes `needs_login`, message still held → sign in → message delivered within 5 s, sender gets the late-delivery notice, `InboxList` shows it delivered. Run once with the sender signed in to MuxBus and once not.
2. Same, with the relay path forced (target on another instance): subscribe-add triggers a pull; with no other traffic the message arrives within 5 s of subscription (#4122 makes this true; add the end-to-end test).
3. Idle `active` and `idle` receivers: `Delivered (active|idle)`, no mailbox row left behind.
4. A live receiver whose CLI reported an auth failure: the jekt is held, not written; after login it is delivered once (no duplicate on `retryLastTurn`).
5. Stop mid-queue: the deferred jekts survive a controller drop and an srv restart; none are lost.
6. Expiry: a held message past 24 h and a relayed one past 30 min each produce exactly one sender notice; a sender that is itself offline gets it on reconnect.
7. Attempt budget: three spawn-gate refusals do not count against the 20; 20 genuine failures produce `failed` and a sender notice.
8. Trust: a replayed message keeps its original verdict (§8); `TIER=sensitive` / `ESCALATE=required` messages are never auto-acted on from a digest.
9. Leases: with the agent live on instance B, a send from instance A is `relayed`, not held on A, and B receives it; after a Take over from B to A, rows B was holding reach A exactly once; with the relay unreachable, A holds locally and the result says `unconfirmed`.
10. Ordering: messages drain oldest first; a live message sent during the drain does not overtake held ones (the durable spec's recorded residual).

## 8. Security

- A mailbox row stores the trust verdict computed **at receipt** (`sig_verified`, `reagent_verified`, `lan_verified`, `channel_verified`, `is_transcript_request`, `transcript_request_escalate_forced`, already columns of `db_jekt_held`). Replay renders that verdict; it never re-derives it from keys that may have rotated. `ESCALATE` is recomputed from the stored fields only.
- A digest keeps each message's own marker. A digest never lowers a tier, and an `ESCALATE=required` message stays addressed to the human; it is listed in the digest as "needs operator confirmation", not summarised away.
- Bodies sit at rest up to 24 h in `objects.db` (as today); delivered bodies are cleared after delivery (§5.3).
- `InboxList` / `MessageStatus` expose only the caller's own mailbox; a sender sees only the state of messages it sent.
- Receipts are `system` jekts with `TIER=info` and are not themselves receipt-eligible. Presence (`needs_login`) is already visible via `DiscoverAgents`; it adds no new disclosure.
- A **peer agent's message is never operator approval** (unchanged rule): held or digest delivery does not change who may confirm an `ESCALATE=required` action.

## 9. Compatibility

- Tool strings keep their first word; the new parts are in parentheses after the name and at the end.
- HTTP responses add fields only.
- `db_jekt_held` gains nullable columns by migration; the old binary still reads the rows it writes.
- A relay that has not learned `expired` behaves as today (rows filtered on read); srv then tells the sender from its own clock when the TTL it set elapses.

## 10. Open questions

- **O1.** What the operator saw on pane open (§1). Reproduce with the pane transcript and a screenshot before Phase 1; it may be a second defect in how a not-yet-written message is displayed.
- **O2.** Does the relay already wake a newly subscribed agent? Read in `agentmux-cloud` (`muxbus/server/src/index.ts`): wakes are sent on new injections only. If the relay can cheaply send one wake on `subscribe:add`, Phase 0.1 shrinks to a server change, but srv should not depend on it.
- **O3.** `read` needs a reliable signal that the model consumed the line. Candidate: the stream-json user-message echo in the next turn's `init`. Not needed until someone wants "read" shown.
- **O4.** Should a sender be able to ask for `no_hold` (deliver now or fail) for time-critical messages (e.g. review requests that go stale, issue #3894)? Proposed: yes, a `ttl_seconds` and `no_hold` argument on `SendMessage`, honoured by held, deferred and relay alike, which also closes #3894.
- **O6.** Should the read-only lease query also report *where* (host and instance) so the send result can say "live on Area54"? Useful for the sender; it discloses presence the relay already shows through `409 held_by`.
- **O5.** How much of the 7-day receipt history is worth keeping, and whether it should be visible in the pane.

## 11. Cost

Phase 0: about a day (three small changes plus tests; the cloud-subscriber change needs the incident test). Phase 1: 2–3 days (migration, moving the deferred queue, auth-state plumbing, drain triggers). Phase 2: 2 days (tools, digest, notices). Phase 3: 1–2 days (UI). Risk is concentrated in Phase 1's auth-state tracking (a wrong `needs_login` would hold messages for a healthy agent) and in drain ordering; both have a test above.
