# SPEC: Agent display naming & addressing across host, LAN, and WAN

**Date:** 2026-08-22
**Status:** Draft — architecture proposal, no code landed. Shared substrate spec:
consumed by, not superseding, its two sibling specs below.
**Scope:** Agent **display naming** only (the human-facing label shown in tabs,
pickers, presence indicators). Does **not** touch `AGENTMUX_AGENT_ID`, jekt
signing identity, or any routing/security identity — those are already solved
and orthogonal to this layer (see §7).
**Related:** `SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21.md` (needs a naming
pattern for forked agents), `SPEC_AGENT_PANE_CROSS_CHANNEL_LAN_WAN_SYNC_2026_08_21.md`
(needs a shared "which peer owns this" concept for discovery/presence),
`SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md` (established the host-global
agent registry this spec's host tier builds on), `SPEC_MULTI_SESSION_AGENT_FORK_2026_06_06.md`
(the existing but under-specified "#2" auto-naming rule), `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`
(the LAN peer-identification this spec's LAN tier reuses). **Cross-repo (agentmuxai/agentmux-cloud,
verified at commit `e0a756e`):** `muxbus/server/src/agent-ownership.ts` (the
`(account_user_id, agent_id)` table this spec's WAN tier anchors to),
`muxbus/server/src/agent-binding.ts` (a *different*, narrower existing use of
"binding" — see §2.3, do not reuse that word for this spec's concepts),
`muxbus/SPEC_AGENT_PUBLIC_ID_2026_06_21.md` (confirms `agent_id` is flat/global/
unnamespaced in muxbus today — the real gap this spec's WAN qualifier
compensates for), `muxbus/PLAN_PER_AGENT_CREDENTIAL_BINDING_2026_07_06.md`
(the abandoned per-agent-credential design; its open-question #2 is directly
relevant to §4.6's proposal)

> **Naming.** "Qualified" vs. "unqualified" name, borrowed deliberately from git
> (`main` vs. `origin/main`) and email (`user` vs. `user@host`) — both are
> well-understood precedents for "a short name works until it might collide, at
> which point prefix/suffix the origin, don't invent a new namespace." The
> per-peer/per-host identifier used for qualification is a **`HostLabel`** (§4.2)
> — deliberately not called a "channel" (already a different, data-isolation
> concept in this codebase, see `SPEC_DATA_CHANNELS_2026_05_24.md`) or an
> "identity" (already the credential/Armory concept).

---

## 1. Problem / TL;DR

Two sibling specs each independently need a notion of "which machine/peer is
this agent actually on": the fork spec, to decide whether a forked agent's
"#N" counter can collide with a fork of the same lineage happening elsewhere;
the mirror spec, to discover and label the peer a pane is being mirrored
to/from. Solving this once, as a shared naming/addressing layer both specs
consume, avoids two independently-invented, subtly-incompatible notions of
"host" appearing in the codebase. This spec also directly answers a concrete
question raised while drafting the fork spec: **if "AgentX #3" exists on one
channel and "AgentX #4" gets minted on a different channel on the same host at
roughly the same time, does the counter still work?** — and extends that
answer outward to what happens once agents/panes are addressed across LAN and
eventually WAN, which both sibling specs anticipate as a "fluid across all
three tiers" end state.

## 2. Current architecture (code-verified)

**Host tier: the registry is already global per-host, not per-channel.**
`SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md` (implemented, load-bearing
per this repo's `CLAUDE.md`) re-rooted agent definitions and instances to
`~/.agentmux/shared/agents/{registry,definitions}/` — one shared store per
**machine**, read/written by every channel and version running on it. So a
fork minted on `stable` and a fork minted on a `local-<branch>-<hash>` dev
channel, on the *same* host, already see and write the *same* underlying
store — this part of "does the count still work" is already true by
construction, not something this spec needs to build.

**But the write primitive is overwrite-safe, not allocation-safe.**
`agentmux-srv/src/registry/atomic.rs`'s `write_atomic` (temp file → `fsync` →
rename-over-target) guarantees no reader ever observes a half-written file. It
does **not** provide exclusive-create or compare-and-swap — nothing prevents
two processes from independently computing "current max is 3" and both
writing a *different* new record (different UUID, different file) both
displaying "AgentX #4". This is a real, code-confirmed gap, not a hypothetical
one: `SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE` §11.5 landmine #2 flags exactly
this class of concern ("verify \[atomic rename\] holds under simultaneous
startup migrations") without fully resolving it for the counter-allocation
case specifically.

**LAN tier already has an implicit host-identification concept, just not
surfaced as a naming primitive.** `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`'s
per-agent Ed25519 verification fetches "the claimed sender's own public key...
from whichever LAN peer hosts that agent" — meaning the system already has to
resolve, for every LAN-addressable agent, which physical peer hosts it. This
resolution is currently internal to the signing/verification path; nothing
today exposes it as a user-facing label.

**WAN tier's identity anchor, verified against `agentmuxai/agentmux-cloud`
at commit `e0a756e` — corrects and sharpens the original draft of this
section.** `MUXBUS_AGENT_ID` mirrors `AGENTMUX_AGENT_ID` at spawn so muxbus
can route to an agent by identity across the WAN (per this repo's `CLAUDE.md`)
— but muxbus's own data model is more specific, and more useful, than a bare
identity mirror:

- **`agent_id` is a flat, global, self-declared string, with no per-account
  namespace** (`muxbus/SPEC_AGENT_PUBLIC_ID_2026_06_21.md`; enforced via
  `normalizeAgentId()`, `muxbus/server/src/index.ts`). **Two different
  accounts' agents named `korp` collide on the exact same routing key today**
  — explicitly flagged as unsolved, out-of-scope Phase-3 work in
  `muxbus/PLAN_PER_AGENT_CREDENTIAL_BINDING_2026_07_06.md`. This spec's WAN
  qualifier isn't cosmetic disambiguation on top of an already-unique id — it
  is compensating for a **real, currently-open collision gap** in muxbus's
  own storage.
- **A real ownership pairing already exists and is queryable:**
  `muxbus/server/src/agent-ownership.ts` — table `muxbus-agent-ownership-{env}`,
  PK `account_user_id`, SK `agent_id`, checked via `isAgentOwnedByAccount()`.
  **Decision: the WAN qualifier is `<account_user_id>/<host-label>`** (or a
  resolved display form, e.g. the account's email), anchored to this
  already-shipped table — not a vaguer "muxbus account" hand-wave.
- **Do not call this a "binding"** — `agent-binding.ts`'s `checkAgentBinding()`
  already uses that word for a narrower, different thing (does *this*
  authenticated request's account match the ownership record for the
  `agent_id` it's sending *as*). Reusing "binding" here would collide
  semantically with already-shipped code.
- **Critical constraint the original draft missed: muxbus cannot resolve
  "host" server-side, at all, today.** WAN delivery is mailbox/poll-based —
  every connected sidecar wakes on a broadcast ping and independently checks
  `GET /reactive/pending/:agent_id` for whatever it locally hosts
  (`muxbus/server/src/index.ts`, `broadcast.ts`). The WebSocket connections
  table is deliberately zero-metadata (`ws-connect.ts`: "$connect only needs
  to know a socket is open — no agent/account ownership is recorded here").
  **There is no host concept in muxbus's data model at all.** Consequence:
  the `host-label` half of a WAN-qualified name must be **asserted
  client-side** (by the sidecar itself, which knows what machine/channel it's
  running on) — muxbus can confirm *ownership* (`account_user_id` ↔
  `agent_id`) but can never independently verify *which host*. This is
  consistent with, not a workaround for, this spec's own G4 ("a qualifier is
  a label, never a trust claim") — a self-asserted host-label was always
  going to be exactly as trustworthy as `TRUST=network-claimed` already is
  for WAN jekt, no more.

**The existing fork auto-naming rule doesn't specify its own scope.**
`SPEC_MULTI_SESSION_AGENT_FORK_2026_06_06.md`'s "Senior Dev" → "Senior Dev #2"
rule predates the host-global registry work and never states whether the
counter is meant to be unique per-channel, per-host, or wider — this spec
closes that gap explicitly (§4.3).

## 3. Design goals

| # | Goal |
|---|---|
| G1 | A short, unqualified name ("AgentX #4") is sufficient for the overwhelming common case — most agents never leave their host |
| G2 | The short name never has to *change* when a boundary is later crossed — a qualifier is appended, non-destructively, never a rename |
| G3 | The fork spec and the mirror spec share exactly **one** host/peer-identification primitive, not two independently-invented ones |
| G4 | A naming qualifier is a **label**, never a trust claim — matches the jekt trust model's own posture that crossing a network boundary never proves identity by itself |
| G5 | Mirroring a pane never mints a new name (§4.5) |

## 4. Proposed design

### 4.1 Three tiers, matching jekt's existing DELIVERY split

Deliberately reuse the exact host/lan/wan three-way split jekt's `DELIVERY`
field already establishes, rather than inventing a fourth taxonomy for naming:

| Tier | Unqualified name valid when | Qualified form |
|---|---|---|
| **Host** | Always, within one host's global registry (§2) | `AgentX #4` (no qualifier shown) |
| **LAN** | Never, once a name is visible to a second machine | `AgentX #4@<host-label>` |
| **WAN** | Never | `AgentX #4@<account_user_id-display>/<host-label>` — account half server-verifiable against `muxbus-agent-ownership-{env}`, host-label half self-asserted (§2) |

The qualifier is appended only once a name actually becomes visible outside
its host of origin (a mirror connects, a fork lands on a remote peer) — a
purely host-local agent never shows one (G2).

### 4.2 `HostLabel` — one shared primitive for both sibling specs

```
HostLabel {
  display: string,     // human-facing — default: OS hostname; user-overridable (§6, open Q1)
  scope: "lan" | "wan", // host tier needs no HostLabel at all — see §4.1
  stable_id: string,    // the actual anchor: LAN peer id (from the discovery already
                         // backing SPEC_JEKT_LAN_TIER_SIGNING's pubkey fetch) for "lan";
                         // self-asserted by the sidecar (muxbus has no host concept
                         // to verify against, §2) for "wan" -- paired with, but
                         // distinct from, the server-verifiable account_user_id half
}
```

For WAN specifically, the full qualifier is **two independently-sourced
halves**, not one opaque id: `account_user_id` (server-verifiable against
`muxbus-agent-ownership-{env}`) and `host-label` (client-asserted,
unverifiable by muxbus, §2). Never collapse these into a single "WAN
HostLabel" value that looks server-verified end to end — it isn't, and
presenting it as if it were would violate G4.

Both sibling specs resolve to this one type instead of each defining their
own notion of "which machine":
- **Fork spec** (`SPEC_AGENT_QUICK_FORK_NEW_TAB`): if/when a fork lands on a
  remote peer rather than the local host (explicitly a non-goal there today,
  called out here as the reason this spec exists ahead of that capability),
  the new instance's qualifier is that peer's `HostLabel`.
- **Mirror spec** (`SPEC_AGENT_PANE_CROSS_CHANNEL_LAN_WAN_SYNC`): its Phase
  B/C discovery sections should resolve peers to `HostLabel` values directly,
  replacing their current ad hoc "LAN peer"/"WAN peer" prose (see §4.5 and the
  patch applied to that spec alongside this one).

### 4.3 Counter allocation, tier by tier (resolves the race question)

- **Host tier:** best-effort scan-and-increment (as `SPEC_MULTI_SESSION_AGENT_FORK`
  already does), explicitly **not** treated as atomic/collision-proof, because
  §2 confirms the underlying write primitive can't provide that. Mitigation:
  pair the number with a short, inherently-collision-resistant suffix — reuse
  the same disambiguator the workspace-folder naming convention already uses
  (a short date+letter tag, or a few hex characters of the instance UUID) —
  so a rare simultaneous double-fork produces `AgentX #4-0822k` vs.
  `AgentX #4-0822p`: momentarily the same *number*, never actually ambiguous
  as *strings*. No new atomic-sequencer infrastructure required.
- **LAN/WAN tiers: do not attempt to synchronize the counter across hosts at
  all.** This is the resolution to "does the count still work across
  channels/hosts": **it doesn't need to** once a name is qualified. `AgentX #4@hostA`
  and `AgentX #4@hostB` are simply different strings the moment they're
  qualified — there is no ambiguity to resolve, so there is no need for, and
  no cheap way to build, a cross-machine sequencer. Treat this as a
  deliberate non-goal (§8), not a gap.

### 4.4 Forking across tiers

`SPEC_AGENT_QUICK_FORK_NEW_TAB`'s v1 only ever lands a fork on the same host
(its own §8 non-goals). Under this scheme, that means v1 quick-forks **never
need a qualifier** — only the host-tier counter+suffix from §4.3. The
qualifier machinery in this spec exists specifically so that *when* (not if,
per the "fluid across all three tiers" framing) a future capability forks
directly onto a LAN/WAN peer, the new instance is unambiguous on arrival —
qualified immediately by its landing peer's `HostLabel`, no retrofit needed.

### 4.5 Mirroring never mints a name

This is the key correction to the mirror spec: **a mirror connection is a new
*viewer* of an existing name, never a new registry entry, never a name
variant.** The mirrored pane's chrome renders the existing (possibly already
host/LAN/WAN-qualified) name plus a **presence list** of active viewers, each
one itself labeled with *its own* `HostLabel` — e.g. "AgentX #4 — also open
from: korp-laptop (LAN)". This reuses `HostLabel` in the opposite direction
from §4.4 (labeling a *viewer*, not an *owner*) but it's the same primitive,
which is exactly the point of extracting it once.

### 4.6 Synergy: this spec's WAN anchor and jekt's unbuilt WAN signing gap

Raised directly by the human while this spec was being written, and worth
making explicit rather than leaving as a coincidence: this spec's WAN
qualifier work and jekt's own known gap — "general agent-to-agent WAN signing
does not exist yet... an arbitrary WAN jekt's `source_agent` remains exactly
as forgeable as before" (this repo's `CLAUDE.md`, `agentmux-cloud` issue
#2586's unbuilt half) — **could plausibly be closed by the same piece of new
infrastructure**, verified against `agentmux-cloud`'s actual code rather than
assumed:

- **The wire format and storage path for a real per-sender WAN signature
  already exist, end to end, proven for exactly one identity.**
  `muxbus/consumers/github/handler.ts` signs every reagent-originated jekt
  with a pinned Ed25519 key and attaches `reagent_sig`/`reagent_key_id`/
  `reagent_msg_id`/`reagent_ts_secs` to the `Injection` record
  (`muxbus/server/src/store.ts`); the receiving agentmux-srv verifies it
  client-side against a pinned public key (`agentmux_common::jekt_sign`).
  Critically, **these fields are generic in the schema** — nothing about
  `Injection`'s shape hardcodes "reagent"; only the one Lambda that currently
  populates them does. The proven, shipped part of "general WAN signing" is
  exactly this: the fields exist, the storage is opaque pass-through
  (`store.ts`: "the server never verifies it"), and client-side verification
  already works for one sender.
- **What's missing is per-agent (or per-account) key issuance and pinning —
  and this spec already needs an identity anchor to hang it on.** The
  abandoned `PLAN_PER_AGENT_CREDENTIAL_BINDING_2026_07_06.md` design's own
  open-question #2 floated, but never built, "a self-issued, KMS-signed
  capability token" per agent. **If the already-shipped `agent-ownership.ts`
  row (`account_user_id`, `agent_id`) grows a public-key field** (or a new
  table keyed identically), the same record this spec's WAN qualifier already
  needs to look up for the *account* half of a display name could *also*
  serve as the trust anchor jekt's WAN signing gap needs — one new field,
  reused for two purposes, instead of two independent designs each inventing
  their own identity record.
- **This does not close the gap by itself** — naming only needs to *display*
  the account/agent pairing; it doesn't need the pairing to carry a
  verifiable signature. Actually closing jekt's WAN gap requires someone to
  build key issuance, rotation, and the `Injection`-population logic
  analogous to `handler.ts`'s reagent path, generalized to any owned agent —
  real work, not a byproduct of this spec. What this spec's research
  establishes is narrower and still useful: **the two problems share a
  natural home for their identity anchor**, so whoever eventually builds
  general WAN signing should extend `agent-ownership.ts` rather than invent a
  parallel identity table this spec would then have to reconcile with.

This is explicitly a **note for whoever picks up jekt's WAN-signing gap
later**, not a phase this spec commits to delivering — see §8 non-goals.

## 5. Decision tables

### 5.1 Why not one global (UUID-based) namespace for every agent, always?

| | **Tiered qualification (this spec)** | Always-global unique names |
|---|---|---|
| Common case (host-only agent) | Clean, short, unqualified (G1/G2) | Every agent always shows a long/opaque qualifier, even when never leaving the host |
| New infrastructure needed | None beyond `HostLabel` (already implicit in LAN signing) | A real global naming authority |
| Matches existing codebase bias | Yes — mirrors the registry's own "cheap, eventually-consistent" design philosophy | No |

**Decision: tiered qualification.** Only pay the naming-complexity cost once a
name actually needs to travel.

### 5.2 Why not build a cross-host atomic counter/sequencer?

| | **No cross-host sequencer (this spec)** | Hosted/distributed sequencer |
|---|---|---|
| New dependency | None | A new always-on service (or LAN consensus protocol) — a new single point of failure for a purely cosmetic concern |
| Solves the actual problem? | Yes — qualification already removes the ambiguity a sequencer would exist to prevent | Also yes, but at much higher cost for no additional benefit |

**Decision: no sequencer, ever.** Qualification is strictly cheaper and
already sufficient (§4.3).

## 6. Phasing

| Phase | Deliverable | Gated by |
|---|---|---|
| **1** | Host-tier fix: pair the existing "#N" auto-naming rule with a collision-safe suffix (§4.3) | Independent — can ship alongside `SPEC_AGENT_QUICK_FORK_NEW_TAB` Phase 1-2 |
| **2** | Formalize `HostLabel` for LAN scope, surfaced from existing LAN peer discovery (no new discovery mechanism, just exposing what `SPEC_JEKT_LAN_TIER_SIGNING` already resolves internally) | Needed once either sibling spec's LAN phase ships |
| **3** | `HostLabel` for WAN scope (`muxbus-account/host-id`) | Needed once the mirror spec's WAN phase (Phase C) ships |

## 7. Relationship to routing/security identity (non-overlap, stated explicitly)

This spec is strictly the **display** layer. It changes nothing about:
- `AGENTMUX_AGENT_ID` (routing identity, tied to the `AgentDefinition` slug).
- Jekt's HMAC (host)/Ed25519 (LAN)/reagent-pinned-key (WAN) signing — a
  `HostLabel` is never presented as, or treated as, proof of anything; it is a
  label a human reads, not a credential a system trusts (G4). A LAN mirror's
  `HostLabel` says "this claims to be korp-laptop" with exactly the same
  epistemic weight `TRUST=network-claimed` already carries for jekt — labeling
  and trust are deliberately kept separate.

## 8. Non-goals

- A global (cross-host) uniqueness *guarantee* for unqualified short names —
  never the goal; qualification is the tool, not elimination of the
  possibility that two hosts both have an "AgentX."
- Any cross-host atomic counter/sequencer (§5.2, explicitly rejected).
- Changing how `AGENTMUX_AGENT_ID` or any jekt signing key is minted or
  scoped — orthogonal (§7).
- User-facing UI for renaming/customizing a `HostLabel` beyond the basic
  override noted in §9 Q1 — a full "manage my host labels" surface is a later
  polish pass, not required for either sibling spec's v1.
- **Building general agent-to-agent WAN jekt signing.** §4.6 identifies a
  shared identity anchor a future signing scheme *could* reuse; this spec
  does not design or deliver that scheme itself — that's issue #2586's own
  scope, not this one's.
- **Turning on `ENFORCE_AGENT_BINDING=true`** (`agentmux-cloud`,
  currently log-only in every environment per
  `PLAN_PER_AGENT_CREDENTIAL_BINDING_2026_07_06.md`) — this spec's WAN
  qualifier reads the same ownership table that check already guards, but
  flipping enforcement on is an independent, `agentmux-cloud`-side decision
  with its own blast radius, out of scope here.

## 9. Open questions

| # | Question | Default/Recommendation |
|---|---|---|
| 1 | Can a user customize their own host's label (vanity name), or is it always the OS hostname? | Default to OS hostname; allow an override in Settings (this is a **Preference**-tier value per `SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE`'s own three-tier data model — channel-scoped, not identity-scoped) |
| 2 | Should the qualified form ever be shown by default, even at host tier, for consistency? | No — only show a qualifier once a name is actually visible beyond its host of origin; an always-qualified UI would violate G1/G2 for the common case |
| 3 | Where does `HostLabel` get persisted/surfaced in the API surface? | Not fully specified here — likely piggybacks on whatever peer-list structure LAN discovery already maintains for `SPEC_JEKT_LAN_TIER_SIGNING`'s pubkey lookups, plus a small new muxbus-account lookup for WAN. Flagged as an implementation detail for whichever sibling spec's LAN/WAN phase ships first. |
| 4 | How is the WAN qualifier's account half actually *displayed* (raw `account_user_id`/Cognito `sub`, or a resolved email/display name)? | Resolve to a display name (email is the obvious candidate, muxbus already has it via Cognito) — a raw `sub` is meaningless to a human reading a presence indicator |
| 5 | Does resolving the WAN account display name require a new muxbus endpoint, or does the dashboard's existing account-lookup path (`SPEC_CLOUD_SERVICES_DASHBOARD_2026_08_18.md`) already expose it? | Check the dashboard spec's account API before building a new one — likely already covered since "my agents" there is already keyed on `account_user_id` |
