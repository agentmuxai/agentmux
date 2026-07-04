# Plan: muxbus cross-channel duplicate delivery

**Status:** Draft — investigation complete, not yet implemented. Written for
review before touching either repo.
**Author:** AgentX
**Date:** 2026-07-04
**Related:** #1916 (MuxBus cross-channel *local* delivery — the mirror-image
problem; same root cause, opposite symptom).

## Problem

Two different AgentMux channels (e.g. a dev build and a portable build, or two
different versions) running on the same host, each with an agent named
`agent_id`, can cause muxbus to **silently deliver the same jekt twice** — once
into each channel's local pane — because nothing arbitrates that name across
channels.

### Why this is possible, not hypothetical

- `agent_id` is a free-text string set at launch (`AGENTMUX_AGENT_ID`),
  validated only for *format* (`sanitize.rs:124-131` — ASCII
  alphanumeric/`_`/`-`, ≤64 chars). There is no uniqueness check anywhere:
  `Handler::register_agent` (`agentmux-srv/src/backend/reactive/handler.rs:102-142`)
  is a per-process in-memory `HashMap` that can't see other channels' agents,
  and `db_agent_definitions`' only `UNIQUE INDEX` is on `slug`, not `name`
  (`migrations.rs:143-171`).
- The product **intentionally sanctions** running two live sessions of the
  same-named agent at once — `SPEC_NAMED_AGENT_CONTINUATION_2026_05_12.md`
  §9: *"Two running instances of same name — Allowed... Users do this rarely
  but it's a real workflow."*
- muxbus credentials are **global across channels** (PR #1750,
  `~/.agentmux/shared/store.db`), so both channels' `cloud_subscriber`
  connects under the identical account.

### The actual failure sequence

1. Channel A and Channel B both have an agent named `agentx` locally active.
   Both call `cloud_subscriber::add_agent("agentx")`
   (`server/agent_handlers/input.rs:411-412`) on their own independent WS
   connection.
2. A jekt arrives for `agentx`. The server broadcasts a zero-metadata
   `{type:"inject_available"}` wake to **every** connected socket on the
   account — there is no per-agent or per-connection routing
   (`index.ts:70-72`: *"routing is broadcast, not per-agent"*).
3. Both A and B poll `GET /reactive/pending/agentx`. This is a plain DynamoDB
   `Query` with no claim/lock (`store.ts:213-225`) — both get the same pending
   injection.
4. Both locally deliver it (`cloud_subscriber.rs:443-466`,
   `handler.inject_message`).
5. Both call `POST /reactive/ack`. `acknowledgeInjections` (`store.ts:232-252`)
   does an **unconditional** `PutCommand` setting `status: "delivered"` — no
   already-delivered guard, so the second ack just silently overwrites the
   same field. Neither side sees an error.

Net effect: the sender's one message reaches two panes, with no signal to
either side that this happened.

### Severity / how urgent is this

- **Not a billing problem** — `GET /reactive/pending` and `POST /reactive/ack`
  have no `consumeQuota` call anywhere (confirmed via full grep of
  `quota.ts`/`index.ts`); only `jekt_messages` (send) and `drone_runs`
  (inject) are metered.
- **Not triggered by ordinary channel sprawl.** Issue #1916's own worked
  example ("15+ per-channel `agents/` registries") is explicitly *stale disk
  residue* — a collided entry's `pid` was dead and `updated_at` was ~6 weeks
  old. Old, non-running channel directories accumulating on disk (a
  documented, separately-tracked cleanup gap — see `CLAUDE.md`'s "Data
  isolation is per-BUILD" section) do **not** trigger this bug.
- The bug requires the narrower, deliberate condition: the *same agent name*
  concurrently live in 2+ channels, **and** a jekt landing in that overlap
  window. Given "two seats of the same name" is described as rare in its own
  spec, this is real but not a five-alarm fire — worth fixing, not worth a
  hotfix.

## Relationship to #1916

Both problems trace to the same gap: nothing arbitrates `agent_id` identity
across channels on one host.

- **#1916 (delivery miss):** Tier-2 local file registry is siloed per-channel
  (invariant I6), so a message to an agent in a *different* channel can't find
  it at all. Proposed fix: a host-global registry at
  `~/.agentmux/shared/reactive-agents/{agent_id}.json`, explicitly scoped to
  Tiers 2/3 (same-host local delivery) — cloud/Tier 4 called out as
  unchanged/out of scope.
- **This doc (delivery duplicate):** Tier 4 (cloud) has the opposite problem —
  it has *no* channel scoping at all, so the same name active in two channels
  gets the *same* cloud-originated message delivered to *both*.

A single shared concept could address both: if #1916's host-global registry
tracked *which channel(s)* currently have a given `agent_id` locally active,
the cloud-delivery race in this doc could be resolved by making the
claim/delivery atomic per *(agent_id)* regardless of which channel's
`cloud_subscriber` gets there first — the registry itself doesn't need to
pick a "winner" channel, only the claim step does. **Recommend implementing
whichever of these ships first with the other in mind** — they likely share
a migration/design review, even if the actual code changes land separately
(one is agentmux-only, this one spans agentmux + agentmux-cloud).

## Proposed fix: move the claim before delivery, not after

The core bug is a protocol ordering problem: today the flow is
**poll → deliver locally → ack**. Ack currently means "mark read," not
"claim." Two pollers can both pass the poll step before either acks, so both
deliver.

Fix: make the **claim atomic and come first**, so only one sidecar ever
proceeds to local delivery.

### Server-side (`agentmux-cloud`, `muxbus/server/src/store.ts` + `index.ts`)

- Change `acknowledgeInjections` (or add a new endpoint, TBD during design) to
  perform a **conditional** DynamoDB update:
  ```
  ConditionExpression: "status = :pending"
  ExpressionAttributeValues: { ":pending": "pending" }
  ```
  on the existing `status` field, atomically flipping it to `"delivered"`
  only if it was still `"pending"`.
- On `ConditionalCheckFailedException`, return a clear "already claimed"
  signal (e.g. `409 Conflict` or `{claimed: false}` in the response body) —
  today's ack has no failure mode to signal at all.
- This must happen **before** the sidecar delivers locally, not after — i.e.
  the call sequence changes from "poll, deliver, ack" to "poll, claim
  (attempt), deliver only if claim succeeded."

### Client-side (`agentmux`, `agentmux-srv/src/muxbus/cloud_subscriber.rs`)

- Restructure `handle_server_msg`'s `InjectAvailable` handling
  (`cloud_subscriber.rs:443-466` area): for each pending injection returned by
  poll, call the claim step *before* `handler.inject_message(req)`. Only
  deliver locally if the claim succeeded; skip silently (this is the expected,
  correct outcome when another seat won the race) if it didn't.
- No change needed to the "two seats" capability itself — both seats can
  still run concurrently; only the *message delivery* becomes exactly-once
  across them, delivered to whichever seat's poll happens to win. That's
  correct from the sender's perspective (the message reached "the agent"),
  even though which physical pane it landed in is arbitrary.

## Scope / non-goals

- **Not** trying to prevent or warn about same-name agents across channels —
  that's an intentional product capability, don't fight it.
- **Not** trying to pick an authoritative "primary" channel for a given
  agent name — the atomic-claim approach makes that unnecessary; whichever
  seat polls first legitimately wins, no arbitration needed.
- **Not** addressing #1916's local Tier-2/3 delivery-miss gap — cross-reference
  only, separate implementation.

## Open questions

1. Should the claim step be a change to `POST /reactive/ack`'s semantics
   (breaking: existing callers currently expect ack to always succeed), or a
   new endpoint (e.g. `POST /reactive/claim`) called before delivery, with
   `/reactive/ack` staying as a separate "mark fully processed" step after
   successful local delivery? Leaning toward a new endpoint to avoid changing
   existing ack semantics for callers that don't need claim ordering (e.g. the
   TS `muxbus-client` package's poll loop, which may have different
   correctness requirements).
2. Does the GitHub PR-review consumer (`consumers/github/handler.ts`) ever
   poll on behalf of an agent name that could collide across channels, or is
   its delivery path exclusively `POST /reactive/inject` (write-only, not
   subject to this race)? If write-only, no client-side change needed there.
3. Worth a regression test that spins up two local `MessageStore` instances
   (or two poll calls in quick succession) against the same injection to
   confirm exactly-once claim semantics before shipping.
