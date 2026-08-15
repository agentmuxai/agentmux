# SPEC: LAN-tier Ed25519 jekt signing

**Date:** 2026-08-15
**Status:** Proposed
**Tracks:** GitHub issue #2586 ("jekt: extend cryptographic signing to LAN tier
and general agent-to-agent WAN traffic"), scoped to the LAN half only per the
issue's own suggested split — general agent-to-agent WAN signing is a separate
future item blocked on the account-scoped Cognito M2M prerequisite
(`SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13.md` §5.1).
**Builds on:** `docs/specs/SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md`
(host-tier HMAC — the pattern this mirrors, asymmetrically),
`SPEC_JEKT_REAGENT_TRUST_RELAXATION_2026_08_14.md` (Ed25519 verification —
the pattern this mirrors, per-agent instead of one pinned service key), and
`SPEC_JEKT_SENSITIVE_TIER_NARROWING_2026_08_15.md` (defines how a verification
*failure*, as opposed to absence, should affect `TIER`).

## 1. Current state (verified against `main` @ `9349e891b`)

### 1.1 What LAN authentication actually proves today

LAN peer discovery is mDNS-based (`agentmux-srv/src/backend/lan_discovery.rs`):
each instance advertises an `instance_id` and, as of PR #2572, a **LAN-scoped**
`lan_key` distinct from the instance's full local `auth_key` (previously the
full key was broadcast — a real prior vulnerability, already fixed). Peer
discovery and agent lookup (`find_agent`, `GET /agentmux/reactive/agent`) use
this `lan_key`.

`POST /agentmux/reactive/inject` — the endpoint that actually delivers a jekt
— is gated by `lan_or_full_auth_middleware`, which accepts **either** the full
`auth_key` **or** the `lan_key`:

```rust
match auth_key {
    Some(key) if key == state.auth_key || key == state.lan_key => next.run(req).await,
    _ => /* 401 */
}
```

This is **instance-level** authentication: it proves the caller holds a valid
credential for *this srv instance*, nothing more. It says nothing about which
agent, on the calling peer instance, actually originated the message —
`source_agent` in the request body is entirely self-declared, unverified, and
(since `lan_key` is one shared value per instance, not per-agent) any process
on the peer instance that can read that key can claim to be any agent name it
likes. This is the literal thing #2586 is asking to close: LAN has an
instance-to-instance credential, but no agent-to-agent one.

### 1.2 A second gap this signing work must not ignore: `delivery_tier` is self-declared

`InjectionRequest.delivery_tier` (`agentmux-srv/src/backend/reactive/types.rs`)
is a plain `Option<String>` field in the deserialized JSON body. A full-text
search of `agentmux-srv/src/server/reactive.rs` outside `#[cfg(test)]` finds
**zero** production assignments to `req.delivery_tier` — nothing server-side
ever sets or overrides it based on how the request actually arrived (which
auth key matched, which socket it came in on, etc.). `handler.rs`'s tier
escalation reads it as-is: `req.delivery_tier.as_deref().unwrap_or("host")`.

Concretely: a caller who successfully authenticates to `/agentmux/reactive/inject`
using the `lan_key` — the ONLY thing distinguishing a "LAN request" from a
"host request" today is a self-reported string in the same JSON body as
`source_agent`. Nothing stops that body from declaring `"delivery_tier": "host"`
instead, which (for an agent name with no host-tier signing key on file) would
land on `TRUST=self-declared`/`TIER=coord` — silently skipping LAN's
`TRUST=network-claimed` labeling entirely, even before this spec's narrowing
is considered.

**This spec fixes both.** Per-agent LAN signing alone would be signing
messages whose *delivery tier itself* isn't trustworthy — closing §1.1
without §1.2 leaves an easy bypass (just claim `host`). §4 below makes
`delivery_tier` a value the server derives from which credential
authenticated the request, not something the client gets to assert.

## 2. Design: per-agent Ed25519 keypair, mirroring the existing patterns

Follow the issue's own recommendation: Ed25519 (asymmetric), not HMAC — LAN is
inherently multi-party (any receiving instance must verify without being able
to forge), which is exactly what a shared secret can't provide and asymmetric
keys can.

### 2.1 Key generation and storage

New table `db_agent_lan_keys` (mirrors `db_agent_jekt_keys`,
`agent_jekt_keys.rs`), one row per locally-registered `agent_id`:

| Column | Type | Notes |
|---|---|---|
| `agent_id` | TEXT PK | lowercased, same convention as `db_agent_jekt_keys` |
| `public_key` | TEXT | base64, 32 bytes (Ed25519 public key) |
| `private_key` | TEXT | base64, 32 bytes seed — **never leaves this srv instance except into that one agent's own `agentmux-mcp` process env**, same guarantee `AGENTMUX_JEKT_KEY` already has |
| `created_at` | INTEGER | unix seconds |

`agent_lan_key_ensure(agent_id) -> (public_key, private_key)`: mint on first
use (race-safe `INSERT OR IGNORE` + re-read, identical pattern to
`agent_jekt_key_ensure`), using `ed25519-dalek`'s `SigningKey::generate` (the
crate is already a dependency — `agentmux_common::jekt_sign` already links it
for `verify_reagent_jekt`).

Injected into that agent's `agentmux-mcp` process env at spawn, alongside the
existing `AGENTMUX_JEKT_KEY`: `AGENTMUX_LAN_KEY=<base64 private key>`. Same
spawn-time-only, never-over-RPC guarantee.

### 2.2 Public key distribution (the "meat of the work" per the issue)

Public keys are, definitionally, not secret — the distribution problem is
*discoverability*, not confidentiality. Mirror the existing `find_agent` /
`GET /agentmux/reactive/agent` pattern exactly, since LAN peer discovery
already solves "how do I find which peer hosts agent X":

- Extend `GET /agentmux/reactive/agent?id=<agent_id>`'s response (currently a
  bare existence check) to include the agent's LAN public key when found.
- `LanDiscovery` gets a `find_agent_lan_pubkey(agent_id)` analogous to
  `find_agent`, with the same 60s cache TTL, same `lan_key`-gated peer query.
- No new mDNS TXT record needed — public keys are per-agent (potentially many
  per instance) and change over an agent's lifetime rarely if ever; a
  pull-on-demand HTTP lookup (already the pattern for locating the agent
  itself) is simpler than trying to fit a growing set of keys into mDNS's
  TXT record size constraints.

**Trust-on-first-use pinning (reagentx P0, revised after implementation
review):** the discovery mechanism above answers "which peer currently
claims to host agent X," but mDNS discovery is itself unauthenticated — any
device on the LAN can advertise its own instance and locally register an
agent under an EXISTING agent's name. Trusting "whichever peer answers the
lookup first" without further anchoring lets an attacker's self-minted
keypair be accepted as authoritative for a victim's `agent_id`, defeating
per-agent signing's entire premise. `db_lan_peer_pubkey_pins` (schema v20,
`storage/lan_peer_pubkey_pins.rs`) pins the FIRST key ever observed for a
given claimed sender; `verify_lan_signature` (`server/reactive.rs`) treats a
later, DIFFERENT key claiming the same `agent_id` as an active red flag
(`lan_verified = Some(false)`, forcing `TIER=sensitive`) rather than
silently trusting the newer answer — the standard SSH-host-key mitigation
for "no PKI available." This narrows the attack to a race on the very
first-ever lookup for a given `agent_id` (an accepted, well-understood TOFU
residual risk), down from "always fully spoofable by anyone who answers
first."

**Fan-out rate limiting (reagentx P1):** `find_agent_lan_pubkey`'s negative
cache is keyed by `agent_id`, so a caller holding only the `lan_key` could
otherwise force a fresh multi-peer network fan-out on every single request
just by varying `source_agent`, ahead of `Handler::inject_message`'s own
rate limiter (which only runs after `verify_lan_signature` returns). A
simple global token bucket (`LAN_PUBKEY_LOOKUP_RATE_LIMIT`, 10/sec) gates
the fan-out itself — not per-`agent_id`, since the abuse pattern is
specifically about varying the id to dodge a per-id cache — failing closed
(same "nothing to check against" outcome as a genuine not-found, never a
verification failure) when exceeded.

### 2.3 Signing (client side — `agentmux-mcp`)

Mirror `sign_jekt`'s call site in `agentmux-mcp/src/main.rs` (the
`AGENTMUX_JEKT_KEY` → `sign_jekt` closure around line 660) with a LAN
equivalent: when the resolved delivery path is LAN (i.e. `SendMessage`
determines the target isn't locally addressable but IS a discovered LAN
peer's agent — this routing decision already exists, it's what feeds
`delivery_tier` today), read `AGENTMUX_LAN_KEY`, sign the same canonical
tuple `signed_material(msgid, source_agent, target_agent, ts_secs, message)`
`agentmux_common::jekt_sign` already defines for host-tier, using Ed25519
instead of HMAC. A missing key must never block sending (same "signature is
best-effort, unsigned still sends" policy `sign_jekt`'s own doc comment
states) — an agent that hasn't been respawned since this ships has no LAN key
yet, same as today's host-tier `TRUST=self-declared` fallback for un-respawned
agents.

### 2.4 Verification (server side)

New `InjectionRequest` fields: `lan_sig: Option<String>` (base64 Ed25519
sig). No new key-id field needed (unlike reagent's WAN scheme, which has
exactly one rotatable signer with multiple valid keys) — LAN verification
always looks up the CLAIMED `source_agent`'s specific public key, same
identity-scoped lookup host-tier's HMAC already does.

`verify_lan_signature(state, req)`, called from `handle_reactive_inject`
alongside the existing `verify_jekt_signature`/`verify_reagent_signature`
calls, scoped to `delivery_tier == "lan"` only (mirrors both existing
verifiers' tier-scoping):

1. No `lan_sig` present → `lan_verified = None` ("not signed," same as
   `sig_verified`/`reagent_verified`'s `None` case).
2. `lan_sig` present, claimed `source_agent` has no public key on file (via
   §2.2's lookup — requires a LAN round-trip, hence async, unlike host-tier's
   local-only check) → treat as unverifiable, `lan_verified = None`. A
   claimed sender this instance has never heard of isn't a "wrong signature,"
   it's "nothing to check against" — same semantics as host-tier's "no key on
   file" case.
3. `lan_sig` present, public key found, signature does NOT verify →
   `lan_verified = Some(false)`. **A real red flag** — someone tried to forge
   a specific agent's identity, not merely omit proof. Same category as
   `SIG=invalid`/`TRUST=unverified`.
4. `lan_sig` present, verifies → `lan_verified = Some(true)`.

### 2.5 Marker and tier wiring

New `TRUST=` value for the marker (`sanitize.rs`'s `wrap_jekt_message`):
`TRUST=lan-verified` when `lan_verified == Some(true)` — keeps
`TRUST=network-claimed` for everything else on LAN (unsigned, unknown-sender,
or failed), so a human/agent reading the marker sees the SAME
proven-vs-claimed distinction WAN's `SIG=` already provides, just spelled as
`TRUST=` since LAN, unlike WAN, has exactly one signer possible per message
(the claimed sender itself) rather than a separate "which service signed
this" question.

`handler.rs`'s tier-escalation block (post
`SPEC_JEKT_SENSITIVE_TIER_NARROWING_2026_08_15.md`) gains:

```rust
// New: an ACTIVE LAN verification failure is exactly as much a red flag as
// SIG=invalid on WAN or TRUST=unverified on host — someone forged a specific
// agent's identity, not merely sent unsigned traffic.
let is_lan_sig_invalid = delivery_tier == "lan" && req.lan_verified == Some(false);
let is_sensitive = is_network_tier_sig_invalid   // WAN SIG=invalid (unchanged)
    || is_lan_sig_invalid                         // NEW
    || is_unverified_sender                       // host TRUST=unverified (unchanged)
    || matches!(declared_tier, Some(JektTier::Sensitive))
    || is_sensitive_message(&sanitized);
```

`TRUST=lan-verified` itself does not need a separate "don't force sensitive"
carve-out the way WAN's rule 1b did — LAN clean-content traffic is *already*
not forced sensitive as of the 08-15 narrowing regardless of verification
status. What this adds is symmetric to WAN: proof of identity doesn't grant
extra trust beyond default (rules 3/4 — declared-sensitive, keyword match —
still escalate on top), it just changes the label from `network-claimed` to
`lan-verified` so a human reading the marker can tell "this LAN message is
cryptographically confirmed to be from who it says" apart from "this LAN
message merely claims to be."

## 3. Fixing the `delivery_tier` self-declaration gap (§1.2)

**Revised after reagentx P0 on the implementing PR** — the original version
of this section also downgraded a "lan" claim authenticated via the full
`auth_key` to "host." That broke same-host forwarding: Tier 2a/2b in
`server/reactive.rs` (relaying to a sibling channel/instance on this same
machine when the target agent isn't found locally) re-authenticates using
the SIBLING's own full `auth_key` — `lan_key` is a peer-to-peer credential
between distinct machines, never shared across same-host channels. A jekt
legitimately authenticated via `lan_key` at its first hop, whose LAN
signature verification correctly detected a forgery
(`lan_verified = Some(false)`, forcing `TIER=sensitive`), would have that
escalation silently discarded on the second hop: `lan_verified` is
`#[serde(skip_deserializing)]` (resets to `None` on every hop by design —
see §2.4), and downgrading `delivery_tier` to "host" there meant
`verify_lan_signature` never got the chance to re-derive it from the
still-present `lan_sig` field.

The actual gap only needs a one-directional fix. The server, not the
client, must determine when a request is *forced* to be treated as LAN.
Concretely, in `handle_reactive_inject`, after auth middleware establishes
which key matched:

- Authenticated via `state.lan_key` → force `delivery_tier = "lan"`,
  regardless of what the body said. This is the actual bypass this section
  closes: without it, a `lan_key`-only holder could self-label a request
  "host" to dodge LAN's stricter verification entirely.
- Authenticated via `state.auth_key` → the body's own `delivery_tier` claim
  is trusted as-is (host, wan, *or* lan) — no downgrade. Falls back to
  `"host"` only when the body omits the field entirely.

**Second revision, reagentx P0 (round 2):** this section originally also
claimed "holding the full local `auth_key` already grants complete control
over this instance, so self-labeling a request's tier grants nothing beyond
what that credential already allows (at worst it triggers MORE scrutiny,
never less)." **That claim was wrong**, and the wrongness was exploitable:
every locally-spawned agent shares the SAME instance-wide `auth_key`
(`agentmux-srv/src/server/agent_handlers/input.rs`), and
`verify_jekt_signature` (host-tier HMAC verification) was gated on
`delivery_tier == "host"`. A request claiming `delivery_tier: "lan"` (or
`"wan"`) for a `source_agent` that IS a real, locally-known agent (one this
instance minted a `jekt_sig` key for) skipped host-tier verification
*entirely* — with no `jekt_sig`/`lan_sig` provided, both
`verify_jekt_signature` (never ran) and `verify_lan_signature` (nothing to
check, unsigned traffic isn't forced sensitive by design) stayed silent,
and the impersonation landed at default `TIER=coord`, fully actionable.
Self-labeling a request's tier is NOT scrutiny-neutral — it's a way to pick
*which* verification mechanism (if any) gets to run.

**Fix:** `verify_jekt_signature` is no longer gated on `delivery_tier` at
all — it runs unconditionally, and only ever does anything when
`agent_jekt_key_load` finds a LOCAL signing key for the claimed
`source_agent`. That's `None` for any genuinely-remote LAN/WAN sender this
instance never spawned (zero behavior change for real network traffic —
the whole point of per-agent keys being minted only for locally-spawned
agents), but it now catches an unsigned or wrong-signature impersonation of
an agent this instance actually knows, regardless of what tier the request
claims to be. This is the real fix; `resolve_delivery_tier` trusting the
body's tier claim for full-`auth_key` callers (needed for legitimate
same-host forwarding, §3 above) is no longer relied on as a security
boundary on its own — verification no longer depends on tier labeling
being honest.

## 4. Failure semantics (matches existing patterns exactly)

No silent drop, no silent trust — same three-way split every other
verification mechanism in this codebase already uses:
- No signature → `lan_verified = None`, `TRUST=network-claimed`, tier per the
  08-15 narrowing (not forced sensitive by this alone).
- Signature present, fails → `lan_verified = Some(false)`, forced
  `TIER=sensitive`, unconditionally.
- Signature present, verifies → `lan_verified = Some(true)`,
  `TRUST=lan-verified`, tier per declared/keyword rules (never auto-relaxes
  below what clean unsigned LAN traffic already gets, same as WAN's rule 1b).

## 5. Scope explicitly excluded (tracked separately per #2586)

- General agent-to-agent WAN signing (needs the Cognito M2M prerequisite for
  cross-account key attribution — #2586's own suggested split).
- Key rotation/revocation UX (out of scope; host-tier HMAC keys don't have
  this either yet — same gap, not new here).
- Any change to `reagent`'s existing WAN signing scheme.

## 6. Key files

- New: `agentmux-srv/src/backend/storage/agent_lan_keys.rs` (mirrors
  `agent_jekt_keys.rs`)
- `agentmux-srv/src/backend/storage/migrations.rs` — new table
- `agentmux-common/src/jekt_sign.rs` — new `sign_lan_jekt`/`verify_lan_jekt`
  Ed25519 functions, reusing `signed_material`
- `agentmux-srv/src/backend/reactive/types.rs` — `lan_sig`, `lan_verified`
  fields
- `agentmux-srv/src/backend/reactive/handler.rs` — tier escalation (§2.5)
- `agentmux-srv/src/backend/reactive/sanitize.rs` — `TRUST=lan-verified`
  marker rendering
- `agentmux-srv/src/backend/lan_discovery.rs` — pubkey lookup RPC (§2.2)
- `agentmux-srv/src/server/reactive.rs` — `verify_lan_signature`,
  `delivery_tier` server-side override (§3)
- `agentmux-mcp/src/main.rs` — client-side signing (§2.3), mirrors the
  existing `AGENTMUX_JEKT_KEY` closure
- `agentmux-srv/src/server/app_api/agent_open.rs` — env injection at spawn
  (mirrors `AGENTMUX_JEKT_KEY` injection)
- `CLAUDE.md` (both this file and the two loose local copies, once merged) —
  document the new `TRUST=lan-verified` value and its tier treatment

## 7. Open questions for review before implementation

1. Should `delivery_tier` server-side enforcement (§3) land as part of this
   PR, or split into its own prerequisite PR? It's a real, independent
   security gap (exploitable today, with or without this spec) but small
   enough to bundle — leaning toward bundling since shipping §2 without §3
   would ship a signing mechanism with a documented, immediate bypass.
2. `find_agent_lan_pubkey`'s cache TTL — reuse `LAN_AGENT_CACHE_TTL_SECS` or
   a separate, longer one (public keys change far less often than which peer
   currently hosts an agent)?
