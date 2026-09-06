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
| `auth_key` | `X-AuthKey` header | Everything under `authed_routes`, via `auth_middleware` — the full API surface | srv, launcher, frontend, and **every agent process** (`AGENTMUX_AUTH_KEY`) |
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

General agent-to-agent **WAN** signing does not exist (issue #2586's unbuilt
half). An arbitrary non-reagent WAN jekt's `source_agent` is exactly as
forgeable as it always was, which is why tier 4's outbound relay
(`muxbus::relay`) sets no verification fields on the echoed marker rather than
claiming any.

## Keeping this current

If you add a credential, a middleware, or a route to a gated set, add the row
here in the same change. A map that is 80% right is worse than none, because it
gets trusted.
