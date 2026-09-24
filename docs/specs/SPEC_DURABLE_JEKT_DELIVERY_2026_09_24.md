# SPEC: durable jekt delivery — a message to an absent agent is held, not dropped

**Date:** 2026-09-24
**Status:** active — Phase 1 designed, revised after an adversarial pass
(§2), and implemented in the same PR: `storage/jekt_held.rs` (objects v40),
`reactive::hold_for_absent_target`, `server/jekt_held.rs` (replay), the
`HELD_FOR` header field and the MCP's `HELD` report. Phase 2 recorded.
**Trigger:** Repo owner: *"the durable jekt messaging was another thing we
couldn't get working."*
**Related:**
- `SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md` — the in-memory turn-boundary queue
  (#3557, #3562) for a **busy** target. This spec covers an **absent** one and
  does not change that queue.
- `SPEC_MUXBUS_DELIVERY_HIERARCHY_2026_06_15.md:327` — "Persistent message
  store on sidecar — in-memory is fine. Lost on restart; users accept this."
  This spec reverses that for one case: a message no tier could take.
- `SPEC_JEKT_DEFERRED_DELIVERY_NO_MIDTURN_INTERRUPT_2026_09_10.md` — never
  built; its ReAgent gaps (drain-all, two locks, loss on exit, false success
  on overflow) are the checklist for §5.

## 1. What happens today (measured on main @ `c9c5a55b9`)

A jekt goes `SendMessage` → `POST /agentmux/reactive/inject` →
`server::reactive::deliver`: signature checks, then this instance
(`ReactiveHandler::inject_message`), then — only on "agent not found" — the
same-host registry, the cross-channel registry, LAN peers and the cloud relay
(`reactive.rs:1065-1303`). The cloud relay runs only when the sender is signed
in to muxbus (`:1329-1338`).

- **Busy target** (a Claude agent mid-turn): queued in memory until its turn
  ends (#3562). Lost on stop, crash or srv restart; logged as "stranded", the
  sender never told (`persistent/input.rs:514`).
- **Registered but not spawned:** the first message starts a turn (#2960).
- **Absent target** — not registered on any tier this srv can reach (closed,
  not yet launched, srv restarting, or a typo): `deliver` returns
  `success: false, error: "agent not found: X"` and **the message is gone**.
  The sender sees "Message delivery failed". Nothing is stored locally
  (`migrations.rs`: the only jekt tables are keys and the work queue).

That last case is the one users hit when they message an agent that is not
running yet, or across an upgrade restart. It is Phase 1.

## 2. Phase 1 — hold what no tier could take

*Revised after an adversarial pass on the first draft (no P0, four P1s: the
stored trust verdicts do not survive a JSON round-trip; replay spent the
global rate limit and flooded the audit ring; "agent not found" also covers
a peer's refusal; the delivered header showed replay time as send time. Its
P2s — name squatting and typos, a LAN caller filling the hold, periodic
senders, idempotency — are folded in too).*

### 2.1 When a message is held — all of these

1. **Full-key, host-tier origin:** the request authenticated with the instance
   key (`ReactiveAuthVia::FullAuthKey`) and its resolved `delivery_tier` is
   `host`. A LAN-key caller, a cross-channel or LAN forward, and WAN delivery
   (which never reaches `deliver`) are never held — `forward_hops` is in the
   body and proves nothing.
2. **The target is a known agent of this channel:** it resolves — by UID, or
   by a name that selects exactly one row (`Store::agents_matching_name`) —
   to a non-template `db_agents` row. The row is held **by its UID**. An
   unknown or ambiguous name keeps today's error: a typo is not held for 24 h,
   and a name another block registers later can never receive the backlog
   (any full-key caller can register any display name).
3. **No tier found a candidate:** the local handler said `agent not found`
   **and** no same-host, cross-channel or LAN lookup found an entry for the
   name (a `candidate_seen` flag set wherever a tier finds one). A target
   that is alive elsewhere but refused — rate limit, queue full, a restarting
   peer — keeps today's error rather than being held here and never replayed
   there. A sender signed in to muxbus never reaches this point for an
   absent name: the cloud relay takes it as QUEUED first (recorded).
4. **Not a periodic sender:** `source_agent == "cron"` is never held — a job
   fires again on its schedule, and a stale backlog of fires would each start
   a turn.

The response is then:

```json
{ "success": false, "held": true, "request_id": "<id>",
  "error": "agent X is not running — held for delivery on this AgentMux instance for up to 24 h" }
```

`success` stays `false` (it was not delivered): an older MCP reports a
failure whose text says it is held; the Phase 1 MCP reports
`HELD for X — not delivered yet; delivered when X starts on this AgentMux
instance (channel) within 24 h.`

### 2.2 The table

`db_jekt_held` in the channel's object store, next schema version:

| column | meaning |
|---|---|
| `request_id` TEXT PK | `resp.request_id` — the handler mints one when the sender sent none, and it is written back into the stored request so the delivered `MSGID` is stable. A second request with the same id while held returns the same `held` response and stores nothing. |
| `target_uid` TEXT | the resolved row's UID |
| `target_agent` TEXT | as addressed, for display |
| `source_agent`, `audit_source_uid` TEXT | the sender's claimed name and attributed UID (M4c-2d) |
| `message`, `priority`, `jekt_tier`, `delivery_tier` | the request as delivered |
| `sig_verified`, `reagent_verified`, `lan_verified`, `channel_verified` INTEGER NULL | the **verdicts** computed at accept time, as explicit columns: on `InjectionRequest` they are `skip_deserializing` (so a client can never set them), so they would not survive a JSON round-trip — a forged message would come back as merely unsigned |
| `sent_at_ms`, `expires_at_ms` INTEGER | accept time; `+ 24 h` |
| `attempts`, `last_error` | replay bookkeeping |

Insert and cap check are one statement under the store lock (an
`INSERT … SELECT … WHERE (SELECT count …) < cap`), run on the blocking pool.
**Caps:** 64 held per target UID, 1000 per channel; past either, today's
error plus `"hold full"` — never a false hold. The local attempt is audited
as the "agent not found" it was; the hold is counted (`jekt.held`).

### 2.3 Replay

A replay pass runs **when an agent registers** (a `tokio::sync::Notify` the
registration paths fire — no store I/O under the handler lock), every 30 s
as a sweep, and once at srv start:

1. Delete expired rows (`jekt.held_expired`).
2. For each target UID with held rows, oldest first, **check presence
   without side effects** (`ReactiveHandler` lookup of the UID's
   registration). Absent → skip: no rate-limit token, no audit entry, no
   attempt counted.
3. Present → rebuild the request from the columns: set the stored verdicts
   directly (never re-verify — a signature is valid for 5 minutes), restore
   `audit_source_uid`, recompute the transcript-request fields
   (`resolve_transcript_request_tier_fields`, as the cloud subscriber does),
   set `held_sent_at_ms`, and deliver through the handler's local path with
   audit outcome `held_delivered`. At most 8 deliveries per pass.
   - success → delete the row (`jekt.held_delivered`);
   - rate limit, queue full, spawn in flight → leave it, **no attempt
     counted** (transient);
   - any other error → `attempts += 1`; after 20 the row is dropped
     (`jekt.held_failed`).

### 2.4 Trust and display

Delivered with exactly the verdicts it was accepted with — never raised, and
a forged one stays forced-sensitive. The recipient must not mistake a late
message for a current one: `InjectionRequest` gains a server-set
`held_sent_at_ms` (`#[serde(skip)]` both ways), and the delivered header
renders `TS=` as the **original** send time plus `DELIVERY=held` and
`HELD_FOR=<duration>`. The audit entry's outcome is `held_delivered`.

### 2.5 Residuals, recorded

- **"Delivered" means handed to the target's delivery path.** If that is the
  turn-boundary queue (#3562, in memory) and the agent stops before its turn
  ends, the message is lost as any queued message is today. Phase 2.
- **Ordering:** a live message sent between the target's registration and
  the replay pass (which the registration wakes) can overtake held ones.
- **First-contact spawn:** replaying into a registered-but-unspawned agent
  starts it, synchronously under the handler lock (#2960's path); the
  8-per-pass limit bounds the burst.
- **At rest:** held message bodies live in `objects.db` for up to 24 h, and
  a pre-migration snapshot can keep them longer. Held rows cannot yet be
  listed or cancelled from the UI (Phase 2).
- **No echo to the sender** on a replayed delivery in Phase 1 (live
  delivery's echo is name-keyed and would reach whoever holds the name by
  then); the sender was told `HELD` at send time.

## 3. Phase 2 (recorded, not designed)

Cross-channel replay (re-signed or explicitly marked as replayed), persisting
the turn-boundary queue on stop/crash, sender receipts by UID, and a UI to
list and cancel held messages.

## 4. Testing (Phase 1)

- Held only when all four §2.1 conditions hold; each one failing alone keeps
  today's error (LAN-key caller, unknown name, ambiguous name, a candidate
  seen on another tier, `cron`).
- Row stores the verdicts as columns: a `sig_verified = Some(false)` message
  is still forced-sensitive after replay; a `Some(true)` one stays verified
  with a stale `ts_secs`.
- Same `request_id` twice → one row; a request with no id gets the minted id
  back and delivers with it.
- Caps: the 65th held message for one target → "hold full".
- Replay: an absent target consumes no rate-limit token and writes no audit
  entry; after it registers, the notify pass delivers with `DELIVERY=held`,
  the original `TS`, outcome `held_delivered`, and deletes the row.
- Rate-limited replay leaves the row with no attempt counted; expiry deletes.
- Restart: rows survive a store reopen and are delivered by the start-up pass.

## 5. Checklist from the 09-10 review

- *Drain-all:* replay goes through `inject_message`, so the target's own
  one-per-turn-boundary queue still applies.
- *Two locks:* the table is only touched by `deliver` (insert) and the replay
  task (read/delete), each statement atomic; no in-memory mirror.
- *Loss on exit:* rows are written before `deliver` returns.
- *False success on overflow:* a full hold is an explicit error.
