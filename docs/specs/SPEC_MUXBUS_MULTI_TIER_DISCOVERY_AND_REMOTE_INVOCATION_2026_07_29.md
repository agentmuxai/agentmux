# Spec: multi-tier discovery + remote API invocation over muxbus

**Status:** Proposed (audit + design)
**Author:** AgentY
**Date:** 2026-07-29
**Related:** `docs/specs/SPEC_MUXBUS_DELIVERY_HIERARCHY_2026_06_15.md`,
`docs/specs/SPEC_MUXBUS_AGENT_DISCOVERY_AND_PERSISTENT_DELIVERY_2026_06_16.md`,
`docs/specs/SPEC_MUXBUS_CROSS_CHANNEL_DELIVERY_2026_07_02.md` (parked,
issue #1916), `docs/specs/SPEC_MUXBUS_CROSS_CHANNEL_DUPLICATE_DELIVERY_2026_07_04.md`,
`docs/specs/SPEC_MUXBUS_MULTI_TENANT_SECURITY_2026_07_06.md`,
`agentmux-srv/src/backend/lan_discovery.rs`,
`agentmux-srv/src/muxbus/cloud_subscriber.rs`.

## Motivation

Prompted by a concrete, mundane failure: unable to tell one AgentMux
window from another on the same shared host (multiple dev/portable
instances running side by side), with no way to reach into a specific
instance from outside it. That's the small version of a bigger, real
question: **how should an agent discover and interact with other
AgentMux instances across three tiers — same host (different
channel/instance), same LAN, and the internet — and can it do more than
send a text message once it finds one?**

This doc is both an audit (what exists today, verified against current
`main`, not stale memory) and a design proposal for closing the gaps.

## Audit: current state, tier by tier

Everything below was verified directly against code on today's `main`,
not assumed from prior specs.

| Tier | What it is | Status |
|---|---|---|
| 1 — in-process | Same sidecar, in-memory `HashMap` lookup, direct PTY/stdin write | **Implemented.** `agentmux-srv/src/backend/reactive/handler.rs` |
| 2 — same host, same channel | File registry at `{data_dir}/agents/{agent_id}.json`, HTTP loopback + `X-AuthKey` | **Implemented.** `server/reactive.rs::handle_reactive_inject`, `backend/reactive/registry.rs` |
| 2 — same host, **different channel** | — | **Confirmed broken, unfixed.** `data_dir` resolves per-channel (`AGENTMUX_DATA_HOME`), so two channels on one host have disjoint registries. Spec exists (`SPEC_MUXBUS_CROSS_CHANNEL_DELIVERY_2026_07_02.md`), filed as **issue #1916 (still open, zero comments, zero linked PRs)**. A worktree exists (`wt-xchannel`) but contains **only the spec doc, zero code** — diffed against its own merge-base, no `.rs`/`.ts` changes anywhere. |
| 3 — LAN | mDNS/DNS-SD (`_agentmux._tcp.local.`) + **UDP broadcast fallback** (port 47891, for networks that filter multicast) | **Implemented, opt-in** (`network:lan_discovery`, default off). `agentmux-srv/src/backend/lan_discovery.rs`, with real unit tests including a live round-trip. TXT record carries `auth_key` **in plaintext** — a documented, intentional tradeoff, but worth revisiting (see Design §4). |
| 4 — WAN/cloud | Persistent WebSocket to `muxbus-ws.agentmux.ai`, zero-payload wake → poll pending → **claim-before-deliver** (atomic, fixes a real duplicate-delivery bug, PR #1959) | **Implemented**, push-based (not poll-only). `agentmux-srv/src/muxbus/cloud_subscriber.rs` |
| Discovery endpoint | `GET /agentmux/discovery` — aggregates `host` (same-channel only), `lan`, `wan.subscribed_agents` (this sidecar's own subscriptions only) | **Implemented, but no `host.cross_channel[]` field** — the 07-02 spec proposed one, never built. |
| Mobile LAN pairing | QR-code pairing + UDP broadcast probe, phone↔desktop | **Implemented** — but in `agentmux-mobile`, and scoped to mobile-finds-desktop, not desktop-finds-desktop or agent-to-agent. |
| Cloud-side enumeration | "What agents/instances exist on the platform/account" | **Does not exist — a stated non-goal.** `cloud_subscriber.rs`'s own doc comment: the server can't correlate a wake signal to any particular agent/account; there's no directory query anywhere. |
| Cloud-side per-agent authorization | Verifying a credential is actually allowed to act as the `agent_id` it claims | **Does not exist yet.** Per-agent M2M credentials just landed client-side (PR #2342, today), but server-side enforcement is still log-only (`ENFORCE_AGENT_BINDING` unset). Any valid muxbus credential can currently inject to/claim/impersonate any `agent_id`, cross-account. |
| **Remote API/RPC invocation** | Calling a specific tool/command on a *different* instance's agent | **Does not exist at all, on any tier.** Every tier carries exactly one payload shape: a text `message` string, delivered as a conversation turn or raw keystrokes. `SendMessage` is explicitly "text injection into the target's active conversation," not a structured call. There is no verb beyond message delivery anywhere in the wire protocol. |

**The load-bearing finding**: the question "would this be through muxbus, query the instance, then run the API on that remote instance" has two independent parts, and only the first is even partially true today. Discovery (finding instances) is real and working at Tiers 2 (same-channel)/3/4. **Invocation (actually calling an API on what you found) doesn't exist as a concept anywhere in the codebase** — muxbus is a message bus, not an RPC bus. Closing the same-host cross-channel gap alone would not get you to "query then invoke"; it would only fix "query then send a hopeful text message."

## Research: how established systems solve this

Five areas, each with concrete prior art (not generic advice):

### Same-host, cross-instance discovery
Docker, VS Code, Chrome DevTools, and tmux all converge on the same
shape: **a well-known local directory holding a socket/pipe + a small
manifest/lock file** (PID, port, endpoint), discoverable by glob, with
the lock file doing double duty as both mutex and liveness record
(Chrome's `DevToolsActivePort`, PipeWire's `.lock`). This is *exactly*
what the parked #1916 spec already proposes (`~/.agentmux/shared/reactive-agents/`,
host-global, TTL + dead-pid sweep) — the design was already right, it
just was never built.

### LAN discovery
mDNS/DNS-SD as primary transport (best library support, works within a
subnet) with UDP broadcast as fallback for filtered networks — which is
precisely AgentMux's existing Tier 3 design. The gap isn't the discovery
mechanism, it's **post-discovery authentication**: Syncthing's model
(self-signed cert per device, pinned by device-ID hash, no CA) is the
strongest concrete example of "discovery finds candidates, a separate
trust step decides who to actually talk to" — worth adopting in place
of (or alongside) the current plaintext `auth_key` in the mDNS TXT
record, since mDNS/broadcast are inherently spoofable and should never
be a trust boundary by themselves.

### Unified multi-tier resolution
Three real patterns for "try tier A, then B, then C":
- **libp2p**: separates *discovery* (finding/announcing peers, multiple
  strategies run together — mDNS, DHT, bootstrap list) from *routing*
  (locating one specific peer), all addressed through a transport-agnostic
  `multiaddr` format.
- **WebRTC ICE**: gathers all candidate types (host, server-reflexive via
  STUN, relay via TURN) into one pool, scored by priority, connectivity
  checks race the best pairs first.
- **Tailscale**: strict sequential fallback (direct → self-hosted relay →
  managed DERP relay), with periodic re-upgrade attempts even after
  falling back.

AgentMux's existing delivery waterfall (Tier1→2→3→4, sequential,
first-hit-wins) already matches the Tailscale pattern. That's a
reasonable, simpler default — no need to adopt ICE-style racing unless
latency to a specific tier becomes a measured problem.

### WAN/relay architecture
Tailscale's core structural idea — **splitting control plane from data
plane** — is the piece muxbus cloud is missing. Its coordination server
distributes a "network map" (who exists, their public keys) as tiny
control messages; DERP is a dumb, blind relay for already-encrypted data
and never sees plaintext, never knows who's talking to whom semantically.
Muxbus cloud today is *only* the DERP half (blind routing) with
**no coordination-server half** (no directory/network-map it can serve
back to a querying instance) — which is exactly why "cloud-side
enumeration does not exist" above. WireGuard's contrast is instructive
too: it deliberately has *no* discovery built in at all, proving that
"pure relay, no directory" is a legitimate, common design choice — but
one that means enumeration has to be bolted on separately if you want it,
not assumed to fall out of the relay.

### Capability-scoped remote invocation
Since there's no RPC layer today, this is the part that most directly
needs new design, not just gap-filling. Strongest fits from research:
- **Cap'n Proto's object-capability model**: holding a reference to a
  remote object *is* the authorization to call it — no separate
  per-call ACL check.
- **NATS JWT subject-scoping**: explicit allow/deny lists per subject,
  plus a "temporary one-time reply permission" pattern (`allow_responses`)
  for request-reply RPC without pre-allowlisting every possible reply
  channel.
- **Temporal vs. Celery, as a cautionary tale**: Celery's default pickle
  serializer executing arbitrary attacker-controlled objects is the
  canonical "message bus becomes an RCE vector" failure. Temporal's
  contrast — explicitly named, typed, pre-registered activities/workflows,
  no dynamic dispatch — is the safe shape: a **default-deny allowlist of
  invokable methods**, not "run whatever's in the message."
- **gRPC's method-level authorization** (proposal A43) and **OpenSSH's
  `command=` key restriction / `ForceCommand`** both reinforce the same
  point from different angles: scope *what* a remote caller can invoke,
  not just *whether* they're authenticated at all.

## Design

### 1. Finish the same-host cross-channel fix first (issue #1916)

The design already exists and matches best practice exactly (validated
above against Docker/VS Code/Chrome DevTools). No new design needed —
just build what's already spec'd:

- `get_shared_data_dir()` rooted at `~/.agentmux/shared` (reuses the
  existing narrow I6 carve-out already used for muxbus creds/trust-center
  — not a new exception to the isolation invariants).
- `reactive/registry.rs` re-rooted there, per-name **list** (not single
  entry) tagged with `channel`, `lookup_all()`, TTL + dead-pid sweep.
- New Tier 2b in `server/reactive.rs`: after a same-channel Tier-2 miss,
  iterate host-global candidates freshest-first.
- `GET /agentmux/discovery` gains `host.cross_channel[]`.
- Dual-read migration window (old per-channel path checked as fallback
  for one release).

This is scoped, understood, low-risk, and has sat parked for 27 days for
no reason other than nobody picking it up. Recommend this ships
independently of everything else below — it's valuable on its own even
before any RPC layer exists (a cross-channel *text message* is still
strictly better than today's silent failure).

### 2. Strengthen LAN trust (plaintext auth_key → pinned identity)

Adopt Syncthing's shape, adapted to what AgentMux already has: mDNS/UDP
discovery stays exactly as-is (it's already the right mechanism per
research §"LAN discovery" above) — it should only ever produce
*candidate* addresses. The `auth_key` currently broadcast in the TXT
record is the trust step, and broadcasting it in plaintext undermines the
existing per-instance `X-AuthKey` model everywhere else in this codebase.
Replace with: each instance has a stable identity keypair (could reuse
whatever the per-agent M2M credential infra from PR #2342 already
establishes, rather than inventing a second identity system), mDNS/UDP
only advertises a public identity + connection info, and the actual
`X-AuthKey`-equivalent exchange happens over the resulting connection,
pinned to that identity. Sequencing note: this is independent of §1 and
§3 and can land in any order relative to them.

### 3. Give muxbus cloud a real (minimal) coordination-server role

Per the Tailscale-shaped gap above: cloud stays a blind relay for actual
message payloads (no change to that trust model, no change to the
existing "server can't correlate wake signals to accounts" privacy
posture), but gains a **separate, explicit directory endpoint** an
instance can query for "what other instances/agents does *my own
account* currently have connected" — deliberately narrower than a
platform-wide directory (matches the existing per-account credential
scoping direction from PR #2342, and avoids reopening the cross-account
enumeration/impersonation risk `SPEC_MUXBUS_MULTI_TENANT_SECURITY_2026_07_06.md`
already flagged as unresolved). This is new server-side scope in
`agentmux-cloud`, not just the `amx` sidecar — flagging that explicitly
since it's a bigger lift than the other items here and probably needs
its own dedicated spec once this direction is agreed on, not a
sub-bullet of this one.

### 4. The actual new thing: a typed, allowlisted RPC layer over muxbus

This is the part that doesn't exist today and needs real design, not
gap-filling. Recommend, in order of what research most strongly supports:

- **Default-deny, explicitly-registered method allowlist** (Temporal
  shape, not Celery shape). A remote-invokable method is a first-class,
  named, versioned thing declared in code — not "whatever JSON happens to
  arrive." Start this allowlist *small and deliberately* — e.g. the same
  read-only surface `DiscoverAgents`/`Layout`/`WhoAmI` already expose
  locally via MCP would be a reasonable, low-risk first target for remote
  invocation (query-only, no state mutation), before ever considering
  exposing anything that changes state on a remote instance.
- **Capability-scoped, not identity-scoped, authorization** per
  invocation (Cap'n Proto / NATS shape): discovering an instance and
  being handed a capability to call *specific* methods on it are separate
  steps — finding something on the LAN or WAN should never itself imply
  permission to invoke anything beyond, at most, an unauthenticated
  "are you there" ping.
- **Reuse the existing tier waterfall as the transport**, don't build a
  parallel one. Once §1/§2/§3 close the discovery-side gaps, an RPC call
  is just a differently-typed payload riding the same Tier 1→2→3→4
  delivery path invocation messages already use — the wire-level
  "waterfall to whichever tier resolves the target" behavior doesn't need
  to change, only the payload's shape and what the receiving side does
  with it (dispatch to an allowlisted handler instead of injecting text
  into a conversation).

This item is the biggest, least like "finish what's already spec'd," and
genuinely security-sensitive (per `SPEC_MUXBUS_MULTI_TENANT_SECURITY_2026_07_06.md`'s
already-flagged, still-open cross-account authorization gap at Tier 4) —
recommend it lands *after* Tier 4's per-agent authorization is actually
enforced server-side (not just landed client-side), not in parallel with
it. Adding a remote-invocation surface before authorization enforcement
exists would make an already-flagged gap materially worse.

## Suggested sequencing

1. Finish #1916 (same-host cross-channel) — spec'd, scoped, independent, valuable alone.
2. Enforce Tier-4 per-agent authorization server-side (already tracked, already flagged as a gap, blocks item 5 below from being safe).
3. LAN trust hardening (§2) — independent, can interleave with 1/2.
4. Cloud coordination/directory endpoint (§3) — bigger lift, own spec, needed before meaningful WAN-tier enumeration.
5. RPC layer (§4) — the real new capability, deliberately sequenced last and gated on #2, since it's the one item that turns a messaging bug into a security-relevant feature if done before authorization is solid.

## Open questions

1. Does `agentmux-cloud` (separate repo) have appetite/ownership for §3's
   directory endpoint work, or does this need to be proposed there
   separately before assuming it's in scope for this doc's sequencing?
2. For §4's allowlist, should the same MCP tool definitions
   (`agentmux-mcp/src/main.rs`) be the single source of truth for what's
   remote-invokable, or does remote invocation need its own, deliberately
   smaller registry independent of what's exposed locally? Leans toward
   "smaller and separate" (matches the "start small and deliberate"
   recommendation above) but worth a product decision.
3. Should §1's fix and §4's RPC layer share the same `~/.agentmux/shared/`
   host-global directory concept, or does host-global registry
   (same-host only) vs. RPC-capability-scope (any tier) warrant staying
   architecturally separate even though they're both "cross-instance"
   concerns? Leans toward separate, since §1 is discovery-only and
   inherently same-host, while §4 spans all tiers.

## Files (anticipated — this doc does not implement)

| File | Relevance |
|---|---|
| `agentmux-srv/src/backend/reactive/registry.rs`, `backend/base.rs` | §1 — host-global registry, `get_shared_data_dir()` |
| `agentmux-srv/src/server/reactive.rs` | §1 — new Tier 2b; §4 — RPC dispatch if payload shape gains a "call" variant |
| `agentmux-srv/src/server/mod.rs::handle_discovery` | §1 — `host.cross_channel[]`; §3 — new directory data if cloud gains it |
| `agentmux-srv/src/backend/lan_discovery.rs` | §2 — identity/pinning instead of plaintext `auth_key` in TXT record |
| `agentmux-srv/src/muxbus/cloud_subscriber.rs`, `agentmux-srv/src/muxbus/agent_credentials.rs` | §3, §4 gating — coordination role, per-agent auth enforcement |
| `agentmux-mcp/src/main.rs` | §4 — candidate source for (or explicit non-source for, per Open Question #2) the RPC method allowlist |
| `docs/specs/SPEC_MUXBUS_CROSS_CHANNEL_DELIVERY_2026_07_02.md`, issue #1916 | §1 — the design to actually build, already correct per this doc's research |
| `docs/specs/SPEC_MUXBUS_MULTI_TENANT_SECURITY_2026_07_06.md` | §4 gating dependency — Tier-4 auth enforcement status |
