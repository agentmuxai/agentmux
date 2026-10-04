# PLAN: jekt routing that works without the relay — local first, the relay as an addition

**Date:** 2026-10-02
**Status:** active — R-1 and R-2 built 2026-10-02 (§6 option C): the relay's holder query is a merged cloud-side change, the client and routing are in the agentmux PR that adds this plan. R-3 built 2026-10-02 as a hint rather than a refusal (§7.2): a merged cloud-side change and the agentmux PR that follows it. R-4 and R-5 are proposed.
**Author:** AgentY, at the owner's request
**Amends:** `docs/specs/SPEC_JEKT_DELIVERY_STATES_AND_MAILBOX_2026_10_01.md` Phase 0 item 2 (G2), and adds the relay-side work that item depends on.
**Related:** `SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md` (the lease), `SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md` (the 24 h hold), `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`, `SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md`, PRs #4122, #4211, #4212.

## 1. The owner's requirements (2026-10-02)

1. **The system works without the relay.** A user who never signs in to the cloud, or whose relay is unreachable, gets complete agent-to-agent messaging on their own machine and LAN: delivery, holding for an absent agent, and sender identity.
2. **The relay adds; it does not decide.** It carries messages to installs on other networks and answers questions only it can answer. Delivery that does not need it never waits on it.
3. **The relay keeps a metadata log** of messages when the user is signed in: who, to whom, when, which route, outcome. Never the message text.

## 2. How a message is routed today

`deliver()` in `crates/srv/src/server/reactive.rs`, for a message to agent `T`. Each step runs only if the one before did not place the message.

| # | Where | How it is found | Result |
|---|---|---|---|
| 1 | This channel (this AgentMux instance) | the handler's registration table | delivered, or `QUEUED` while T starts/restarts/stops, or held for sign-in (#4212) |
| 2a | Same computer, same channel, another srv | data-dir registry file | forwarded over loopback |
| 2b | Same computer, another channel | host-global shared registry | forwarded over loopback |
| 3 | LAN | mDNS, and UDP broadcast since #4230 | forwarded to the peer |
| 4 | Cloud relay | none: the relay accepts any name | `QUEUED via the cloud relay`, kept 30 min |
| 5 | Local hold | T resolves to an agent defined here | `HELD (not_running)`, kept 24 h |

Steps 1–3 and 5 need no cloud. Step 4 runs only when the sender is an agent signed in to muxbus (`relay_token`), and is skipped for a message that itself arrived over the WAN.

What the relay hears today: only step-4 messages, and which agents each install has subscribed (`cloud_subscriber.add_agent`/`remove_agent` on pane register/unregister, plus each subscribed agent's lease claims and renewals). A message placed by steps 1–3 never reaches it.

Identity without the relay: `host-verified`, `channel-verified` and `lan-verified` are all checked locally (LAN public keys are fetched from the peer that hosts the agent). Only `wan-verified` needs the cloud (the account's key directory).

## 3. Problems

- **R1. The relay comes before the local hold.** An agent defined on this computer whose pane is closed gets the relay's 30 min expiry instead of the 24 h hold, the sender reads `QUEUED via the cloud relay` instead of `HELD`, and #4212's hold-for-sign-in never applies to it. The 24 h hold is reached only when the sender is *not* signed in. This is the 2026-10-01 incident path (spec §1).
- **R2. The relay accepts any name.** A misspelt or deleted agent's message sits 30 min and disappears; the sender was told `QUEUED`.
- **R3. No record.** Nothing records a message's route or outcome across installs; the planned inbox and receipts (spec Phase 2) have nothing to read.
- **R4. The LAN has no account boundary.** Discovery and forwarding accept any AgentMux on the network (the LAN key is broadcast in the mDNS record by design, `config.rs`); `lan-verified` proves which peer's agent signed, not that the peer is one of the user's machines. Out of scope here (§10), recorded because the owner asked how LAN identity works.

## 4. What srv knows about where an agent lives

| Source | Covers | Limits |
|---|---|---|
| Handler registrations | panes open in this channel | — |
| Data-dir and shared registries | panes open on this computer | — |
| LAN discovery | panes open on LAN peers | only peers discovery reaches (firewall, mDNS; #4230 adds broadcast) |
| Agent definitions (`~/.agentmux/shared/agents/definitions`) | agents created on this computer, every channel | **not synced between computers**; an agent created on two computers (or moved with Take over) is defined on both |
| `wan_lease` cache (`held_elsewhere`) | "another install holds T's lease" | filled only while T is subscribed *here* (claims/renewals); fresh for 90 s; so for a closed pane it is almost always empty |
| The relay | which install holds T's lease (lease store), which installs subscribed T | srv can claim, take, renew and release a lease, **not read who holds it** |

The gap that matters: for **T defined here, pane closed here, and live on another computer over the WAN**, nothing local says so. Steps 1–3 miss it, the definition says "here", the lease cache is empty. Today step 4 delivers it there by accident of order: the relay comes first.

## 5. Target routing

For a message to `T` from a sender here:

1. Steps 1–3 unchanged (channel → computer → LAN).
2. **T is defined here, not found by 1–3:**
   - if T is known to be live on another install (§6), send via the relay;
   - otherwise **hold here** (24 h; `not_running`, or `needs_login` per #4212).
   - Not signed in, or relay unreachable: always hold here.
3. **T is not defined here:**
   - signed in: send via the relay; once R2 is fixed, an unknown name is refused and the sender reads `failed (not_found)`;
   - not signed in: `failed (not_found)`.
4. **Signed in:** report the message's metadata to the relay log (§7.3), whatever route it took, after the fact and never blocking delivery.

## 6. The open decision: T defined here, maybe live on another computer

| Option | Rule | Without relay change | Risk |
|---|---|---|---|
| **A. Hold first** | hold whenever T is defined here, unless the 90 s lease cache says elsewhere | yes | **regression:** a message for an agent defined on both computers and live on the other is held here (24 h, delivered only if it opens here) instead of relayed as today |
| **B. Hold and relay** | hold here *and* post to the relay; the receiver delivers one copy | needs dedup by the original message id across the relay (the relay replaces the id today, `try_cloud_relay`) | duplicates if dedup misses; more relay traffic; two copies to expire |
| **C. Ask the relay** (recommended) | signed in: ask "who holds T's lease?"; another install → relay; this one or nobody → hold. Unreachable or not signed in → hold | **no**: needs `GET /agents/lease/:agent` (§7.1) | a round trip per message to an absent defined agent (cacheable for the lease's 60 s TTL) |

**Chosen: C** (owner: "proceed with your recommendations", 2026-10-02), with A's behaviour as its offline fallback, which is requirement 1 anyway. A was not shipped: it would regress the two-computer case. Against a relay without the route (404, e.g. prod before promotion), C keeps today's order, so the client is safe to release before the relay is promoted.

Also possible without relay change, as a narrowing of A: hold first only for T whose shared registry record (`registry/{uid}.json`) shows its last live instance on this computer. Not proposed: that record's last-writer semantics across computers are not verified.

## 7. Relay changes

This section states the contract srv relies on; the relay side is designed in the private cloud repo.

### 7.1 Read-only lease holder query

`GET /agents/lease/:agent` → `{ "held": bool, "holder": { "instance_id", "host", "channel", "version" } | null, "expires_at" }`. Same auth as `/reactive/pending` (the per-agent or account credential). No side effect: it must not claim, renew or extend. srv caches the answer for at most the lease TTL (60 s) next to `wan_lease`'s existing cache.

Do not emulate it with claim-then-release: a transient claim by an install where T is not running could fence a real start elsewhere (the newcomer-yields rule, `SPEC_AGENT_SINGLE_LIVE_INSTANCE` D1).

### 7.2 Tell the sender when the target is not one of theirs (was: refuse unknown names)

**Revised 2026-10-02, owner-approved.** Refusing names was dropped for two reasons:

- the relay can only say "some account owns this name", so refusing names no account owns would let anyone probe which agent names exist in other accounts;
- messages between accounts are legitimate (the GitHub consumer), so it cannot refuse names outside the sender's own account either.

Instead the relay still accepts every message and `POST /reactive/inject` answers `target_in_account`: whether the **sender's own account** owns the target. It is left out when the relay can't say, never a confident `false`. srv passes it through, and `SendMessage` appends *"No agent of that name has signed in from your account yet, so check the spelling. It is still delivered if one of your agents with that name signs in within 30 minutes, or if another account has an agent with that name."* (Not "only another account": one of the sender's own agents that has never run while signed in also reads `false` and can still receive it; Codex P2 on #4249.) to the relay answer on `false`.

An agent counts as the account's once it has run while signed in (its per-agent credential has been provisioned). That includes agents started before the user signs in, since srv subscribes and provisions every running agent at login (#4122 seeding, `cloud_subscriber`). Defined-but-closed agents are deliberately not provisioned at login: each new agent id counts against the account's provisioning quota. So the hint can be wrong for an agent that has never run while signed in, which is why it reads as a hint and the message is still sent.

### 7.3 Metadata log

`POST /reactive/log` (batched), one record per message:

| Field | Notes |
|---|---|
| `msg_id` | the sender's id (`id=` in `SendMessage`) |
| `from`, `to` | agent ids |
| `from_install`, `to_install` | instance ids (`wan_lease::instance_id`), when known |
| `sent_at_ms` | |
| `route` | `channel` · `host` · `channel_peer` · `lan` · `relay` · `held` |
| `outcome` | `delivered` · `queued` · `held(reason)` · `relayed` · `failed(reason)`, later updated by `delivered_late` · `expired` |
| `trust` | the receiver's TRUST/SIG verdict when known |

Never the message text, its length or a hash of it. Best effort: a background queue in srv, dropped when the relay is unreachable, capped; delivery never waits on it. Retention and who can read it (the account owner, in the app) to be set by the relay owner. This is the record spec Phase 2's receipts and `MessageStatus` can read from across installs.

## 8. What the sender is told (unchanged words, #4211/#4212)

`Delivered to T …` · `QUEUED for T — their agent is starting up …` · `HELD for T (not_running|needs_login) … id=` · `QUEUED for T via the cloud relay (unconfirmed, expires in 30 min) … id=` · `Message delivery failed (not_found|needs_login): …`. §5 changes which one a given case gets: a same-computer absent agent moves from `QUEUED via the cloud relay` to `HELD`.

## 9. Phases

| Phase | What | Where | Depends on |
|---|---|---|---|
| R-0 | This plan; spec Phase 0 item 2 points here | agentmux docs | — |
| R-1 | Hold-first for T defined here (§5.2) with §6 option C; relay only per §6 | `reactive::route_for_absent` | built |
| R-2 | `GET /agents/lease/:agent` (§7.1) | cloud relay (merged) and `wan_lease::holder()` | built; prod promotion pending |
| R-3 | "Not one of yours" hint (§7.2) | cloud relay (merged), srv relay pass-through, `send_message_outcome` | built; prod promotion pending |
| R-4 | Metadata log endpoint and srv's background reporter (§7.3) | agentmux-cloud, then srv | relay owner; retention decision |
| R-5 | LAN account boundary (§10) | separate spec | owner discussion |

## 10. Not in this plan: the LAN account boundary

Any AgentMux on the same network is discovered and can forward messages to this one's agents; their messages are `network-claimed` unless a LAN signature verifies, and a verified one proves the peer's agent, not the peer's owner. On a home network that is the intended behaviour; on shared networks it is an exposure. Options for a later spec: pairing LAN peers (accept only installs the user approved, like `INSTANCE_STATUS=approved` for WAN), or scoping discovery to installs of the same account when signed in, with pairing as the offline path.

## 11. Tests (for R-1)

1. T defined here, pane closed, sender signed in: `HELD (not_running)`, a `db_jekt_held` row, no relay call; T opens → delivered once.
2. Same, sender not signed in: identical result (requirement 1).
3. T defined here, lease held by another install (C: relay answer; A: fresh cache): relayed, no hold row.
4. T not defined here, signed in: relayed (today's behaviour); not signed in: `failed (not_found)`.
5. T defined here, refused by its spawn gate: `HELD (needs_login)` (#4212 path unchanged).
6. Relay unreachable or 404 on the lease route: hold (fail toward local).
7. A message that arrived over the WAN for an absent local agent: unchanged (not held; spec D2).
8. Cron sender, LAN caller: never held, as today.

## 12. As built (R-1, R-2)

- **Relay** (cloud side): `GET /agents/lease/:agent_id`, header `X-Agent-Instance`. Answers `free` | `yours` | `other` + `publicHolder` | `unknown`. A read only; fails open to `unknown`.
- **Client** (`wan_lease::holder`): asks as the message's sender (its relay credential and agent id) about the target's slug, the agent's `AGENTMUX_AGENT_ID`, which its lease is keyed by. Answers cached 20 s; "unreachable" never cached; a 404/405 backs off 10 min. Never touches this instance's own lease state.
- **Routing** (`reactive::route_for_absent`, before `try_cloud_relay`): not holdable (LAN caller, cron, no id) or not defined here → relay first, as before. Defined here: this instance's own fresh "held elsewhere" (`wan_lease::held_elsewhere`) → relay; no source agent or not signed in → hold; else the relay's answer: `free`/`yours`/unreachable → hold; `other`/`unknown`/unsupported → relay. The hold after the relay stays as the fallback.

## 13. Open questions for the owner

1. Promote the R-3 relay change to prod. The R-2 relay change was promoted 2026-10-02 21:39 UTC.
2. Metadata log (R-4) retention, and whether other installs of the account may read it or only the owner in the app.
3. R-5: should LAN discovery be limited to the user's own installs?
