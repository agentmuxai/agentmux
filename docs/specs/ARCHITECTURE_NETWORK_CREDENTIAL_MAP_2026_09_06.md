# Architecture: network credential → route map

**Status:** Living
**Date:** 2026-09-06
**Author:** Agent2
**Origin:** `docs/reports/REPORT_NETWORK_ARCHITECTURE_DRYNESS_AND_ROBUST_LAN_2026_09_06.md` §8

Several distinct secrets appear across srv's network surface. Each has a real,
well-documented reason to exist, and this document is **not** an argument to
consolidate them — scope separation is exactly right for a security boundary.
The gap it fills is narrower: there was no single place saying *which credential
gates which route at which tier*. `Config::lan_key`'s doc comment was the
closest thing and only covered its own case.

## The distinction that matters most

Two different things are easy to conflate, and conflating them is how you end up
reasoning about the wrong control:

- **Route gates** decide *whether a request is allowed to reach a handler at
  all*. They are HTTP-level, checked by middleware, and know nothing about
  message content.
- **Message authentication** decides *who sent a particular jekt*. It is
  evaluated **after** the route gate has already let the request in, and it
  never grants or denies access — it only sets the `TRUST=` field the receiving
  agent sees.

A valid `lan_key` gets a peer through the door; it says nothing about who sent
the message it carries. That is what `lan_sig` is for.

## Route gates

| Credential | Transport | Gates | Held by |
|---|---|---|---|
| `auth_key` | `X-AuthKey` header | Everything under `authed_routes`, via `auth_middleware` — the full API surface, including `GET /agentmux/reactive/history/search` (see the note below it) | srv, launcher, frontend, and **every agent process** (`AGENTMUX_AUTH_KEY`) |
| `lan_key` | `X-AuthKey` header | Exactly two routes, via `lan_or_full_auth_middleware`: `POST /agentmux/reactive/inject` and `GET /agentmux/reactive/agent` | srv only, plus whoever receives the mDNS TXT record it is broadcast in |
| `host_reg_secret` | `host_ipc.Register` argument | That one call — nothing else | srv and the paired CEF host only; **never** an agent |
| `ipc_token` (+ `ipc_port`) | Pushed to srv by the host; replayed by srv when proxying | `/agentmux/browser/*` **on the host's own IPC server**, backing the `/api/v1/ui/{screenshot,click,query}` proxy routes | The CEF host generates it for itself and is the sole source of truth |
| muxbus account token / per-agent M2M credential | `Authorization: Bearer` + `X-Agent-ID` | The **cloud's** routes (`/reactive/inject`, `/reactive/pending/*`, `/reactive/ack`, `/reactive/release`, `/agents/provision`) — outbound only; gates nothing on this srv | Stored in `AppState::id_store`; see the note below |

### Why `lan_key` exists separately from `auth_key`

`lan_key` is minted fresh per launch and is broadcast in the mDNS TXT record, so
anything that can receive a multicast packet on the LAN can read it. Before it
existed, that broadcast carried the full `auth_key` — meaning a passive LAN
listener gained standing access to the entire `/agentmux/service` surface. The
scoped key shrinks a captured value's blast radius to "can forward jekts to this
instance, and can ask which agents live here." See
`SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13.md` §2.1/§3 LAN P0-1.

The two-route set is deliberately kept **out** of `authed_routes` and merged at
the top level with its own middleware, rather than nested — nesting would put
`route_layer(auth_middleware)` on the outside and reject the LAN key before the
inner layer ever saw it.

### `history/search` reads conversation content, and `auth_key` cannot say who is asking

`GET /agentmux/reactive/history/search`
(`SPEC_AGENT_HISTORY_SEARCH_2026_09_17.md`) searches an agent's own past
sessions from disk — strictly more than `/reactive/transcript`, which returns
only the live session's tail. It is deliberately a **full-auth** route, never
in the `lan_key` set: conversation content is exactly what a captured LAN
credential must not reach.

**Whose history is searched comes from the per-agent token, never from a
name.** `auth_key` is shared by every locally-spawned agent, so it cannot say
which agent is calling — the same property that already makes
`/reactive/transcript` readable for any agent by any local caller holding the
key. Since identity M1a every agent process also carries its own
`AGENTMUX_AGENT_TOKEN`, which the MCP sends as `X-Agent-Token`; the route
searches that token's row's history (M4c-2c) and, since 2026-09-24, **refuses a
request without one (403)** rather than resolving the self-declared `agent`
parameter, which is kept only for the actor counters. "Own history only" is
therefore as strong as the token's secrecy: it lives in the agent's own
process environment, never in `.mcp.json` or any other agent's env.
`host_reg_secret` below is the older precedent for "`X-AuthKey` alone cannot
distinguish callers that share it."

### Why `host_reg_secret` exists on top of `auth_key`

Agents share the instance-wide `auth_key`, so `X-AuthKey` alone cannot
distinguish the paired CEF host from any agent process running under it.
`host_reg_secret` is the thing only srv and the host know. When it is unset,
`handle_register` refuses **every** registration rather than accepting one
unauthenticated — an absent secret means there is nothing to check against, not
that checking is optional.

### Where muxbus credentials live

`AppState::id_store` — the same store `CloudSubscriber::init_global` and the
`muxbus.login` / `status` / `disconnect` handlers write to. Not `wstore` (a
per-channel store, where the lookup silently finds nothing whenever the shared
root resolves), and not `identity_store` (which `isolated_auth_enabled()` can
redirect independently of `id_store`, so a reader using it can diverge from the
writer). Enforced by `scripts/check-muxbus-credential-store.sh`; the regression
that motivated the gate is PR #3023.

## Message authentication

None of these gate a route. All three are evaluated after the request is
already in, and only affect the `TRUST=` field on the marker the receiving agent
reads. The authoritative rules for how each maps to `TIER=`/`ESCALATE=` are in
CLAUDE.md's "Jekt security rules" section and the specs it cites — this table
only says what each signature *is*.

| Signature | Tier | Scheme | Proves |
|---|---|---|---|
| `jekt_sig` | host | HMAC-SHA256, per-agent key (`AGENTMUX_JEKT_KEY`, injected into that agent's MCP process env only) | The claimed `source_agent` really sent it → `TRUST=host-verified` |
| `lan_sig` | LAN | Ed25519, per-agent keypair; the public half is fetched from whichever LAN peer hosts that agent | Same claim, across the machine boundary → `TRUST=lan-verified` |
| `reagent_sig` | WAN | Ed25519 against a **pinned service** key | Only that the message came from AgentMux's own GitHub-review service — not per-agent identity → `SIG=verified` |
| `wan_sig` | WAN | Ed25519, per-agent keypair (`db_agent_wan_keys`), **bound to the sending instance** (`source_host` + `source_channel`) | *Nothing yet — see below.* Intended: the claimed `source_agent`, on the claimed instance, under the account the cloud resolved → `TRUST=wan-verified` |

**General agent-to-agent WAN signing is half-built as of 2026-09-17** (issue
#2586's other half, `SPEC_JEKT_WAN_TIER_SIGNING_2026_09_17.md`). Read that row
carefully, because the honest status is narrower than its existence suggests:

- **Signing is live** (PR #3304, #3306). Agents mint a WAN keypair at spawn
  and attach `wan_sig` to every outgoing jekt.
- **Verification does not exist.** Nothing reads `wan_sig`; there is
  deliberately no `wan_verified` field yet, and no `TRUST=wan-verified` value
  is ever rendered.
- **So an arbitrary non-reagent WAN jekt's `source_agent` remains exactly as
  forgeable as it always was**, and tier 4's outbound relay (`muxbus::relay`)
  still sets no verification fields on the echoed marker rather than claiming
  any. Nothing about the trust a reader should place in a WAN jekt has changed.

Signing shipped first on purpose: an agent only receives its key when spawned,
so a verifier landing before keys propagate would apply to almost no live
agent. Verification is blocked on tenant-scoped injection storage (that spec's
§2.1/W2) and on the signed msgid/timestamp surviving the cloud round trip
(§3.4.1) — `cloud_subscriber` currently replaces `request_id` with the cloud's
own injection id and leaves `ts_secs` unset, so a verifier wired today would
fail every legitimate signature.

The key is per **instance**, not per agent name (§2.1.2): one account can run
one agent name on several machines, and on several build channels of one
machine, each with its own database and therefore its own keypair.

| Credential | Transport | Purpose |
|---|---|---|
| `AGENTMUX_WAN_KEY` | agent's own MCP process env, injected at spawn | The private half of that agent's WAN keypair on this instance. Same never-over-RPC, never-another-agent's-env guarantee as `AGENTMUX_JEKT_KEY` / `AGENTMUX_LAN_KEY`. Gates nothing — it signs. |
| `AGENTMUX_HOST_LABEL` | same | Not a credential: the host half of this instance's identity, bound into `wan_sig`'s signed material so a signature can't be replayed as the same agent on a different instance. Paired with `AGENTMUX_CHANNEL`. |

## Keeping this current

If you add a credential, a middleware, or a route to a gated set, add the row
here in the same change. A map that is 80% right is worse than none, because it
gets trusted.
