# SPEC: durable jekt delivery — a message to an absent agent is held, not dropped

**Date:** 2026-09-24
**Status:** draft — Phase 1 designed here, to be built in the same PR after an
adversarial pass; Phase 2 recorded.
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

### 2.1 When a message is held

In `deliver`, when the final outcome is `success: false` with an error
starting `agent not found` **and** the request is local in origin
(`forward_hops == 0` — a forwarded hop is the originating srv's to hold, never
the peer's), the request is written to `db_jekt_held` and the response
becomes:

```json
{ "success": false, "held": true, "held_id": "<request_id>",
  "error": "agent not found: X — held for delivery here for up to 24 h" }
```

`success` stays `false` — it was not delivered — so a pre-Phase-1 MCP still
reports a failure, with a message that says it is held. The Phase 1 MCP
reports `HELD for X — not delivered yet; delivered if X registers on this
machine within 24 h.` Only the absent case is held: an ambiguous name, a
rate limit, a queue-full or a delivery error keep today's response, since
retrying them blindly could deliver to the wrong agent or amplify a fault.

### 2.2 The table

`db_jekt_held` in the per-channel object store (`mstore`), next schema version:

| column | meaning |
|---|---|
| `request_id` TEXT PK | the MCP's msgid — **idempotency**: a retried send with the same id returns the existing row's `held` response, never a second row |
| `target_agent` TEXT | as addressed (name or UID) |
| `request_json` TEXT | the `InjectionRequest` **after** verification: carries `sig_verified`, `reagent_verified`, `lan_verified`, `channel_verified`, `delivery_tier`, `jekt_tier`, `priority`, `source_agent`, message |
| `audit_source_uid` TEXT | the sender's attributed UID (M4c-2d), `''` when Unattributed — kept apart because `InjectionRequest` never serializes it |
| `created_at`, `expires_at` INTEGER | ms; `expires_at = created_at + 24 h` |
| `attempts` INTEGER, `last_error` TEXT | replay bookkeeping |

**Caps:** at most 64 held per target (folded name) and 1000 per channel; past
either, the response is today's failure with `"hold full"` in the error —
never a false hold.

### 2.3 Replay

One background task (installed after `AppState`, like cron delivery) runs
every **5 s** while any row exists, and once at srv start:

1. Delete rows past `expires_at` (log + `jekt.held_expired` counter).
2. For each row, oldest first: rebuild the request from `request_json`,
   restore `audit_source_uid`, and call `ReactiveHandler::inject_message` —
   the **local tier only**, and **no re-verification**: the stored outcome is
   authoritative (a signature is valid for 5 minutes, `JEKT_SIG_MAX_AGE_SECS`,
   so re-verifying a held message would mark every one forged).
   - success → delete the row, count `jekt.held_delivered`, and echo to the
     sender as a live delivery does (`echo_jekt_to_sender`, best effort);
   - `agent not found` → keep (the target is still absent);
   - any other error → `attempts += 1`, `last_error`; after 20 attempts the
     row is dropped and counted `jekt.held_failed` (a target that is present
     but keeps refusing is not "absent").

Registration by any path (HTTP register, persistent spawn, register-tail) is
picked up by the next tick — no hook into the handler lock, which forbids
store I/O (`persistent/spawn.rs` registration runs under it).

Delivery then follows the target's own rules, including the turn-boundary
queue for a busy Claude agent (the replay is ordinary automated traffic).

### 2.4 Trust and display

A held message is delivered with exactly the trust it was accepted with —
never raised. It gains no new field; its audit entry's `outcome` is
`held_delivered` so the Warden audit view can tell it apart. The recipient
sees it as the jekt it was, arriving late; the message header already carries
the send timestamp.

### 2.5 What Phase 1 does not do

- **Other channels / LAN.** A held message is replayed only into this
  instance. If the target later comes up in another channel, it is not
  forwarded (the stored signatures would be stale at the far side, which
  would mark it forged). Phase 2.
- **Stranded turn-boundary messages.** The #3562 queue holds encoded stdin
  lines with no sender metadata; persisting those needs the queue to carry
  the request. Phase 2.
- **Delivery receipts** to a sender that has since stopped.

## 3. Phase 2 (recorded, not designed)

Cross-channel replay (re-signed or explicitly marked as replayed), persisting
the turn-boundary queue on stop/crash, sender receipts, and addressing held
rows by canonical UID once #3497 lands.

## 4. Testing (Phase 1)

- `deliver` holds an absent-target jekt (row written with the verified trust
  fields and `audit_source_uid`), returns `held: true`, and does not hold an
  ambiguous name, a rate limit or a forwarded hop.
- Same `request_id` twice → one row, same response.
- Caps: the 65th message to one target is refused with "hold full".
- Replay: a row for an absent target stays; after the target registers, one
  tick delivers it with the stored trust (`sig_verified` unchanged even though
  its `ts_secs` is stale), deletes the row, audits `held_delivered`.
- Expiry deletes; 20 non-absent failures drop.
- Restart: rows survive a store reopen and are delivered by the start-up pass.

## 5. Checklist from the 09-10 review

- *Drain-all:* replay goes through `inject_message`, so the target's own
  one-per-turn-boundary queue still applies.
- *Two locks:* the table is only touched by `deliver` (insert) and the replay
  task (read/delete), each statement atomic; no in-memory mirror.
- *Loss on exit:* rows are written before `deliver` returns.
- *False success on overflow:* a full hold is an explicit error.
