# SPEC: Cross-channel / LAN / WAN agent pane sync (mirrored panes)

**Date:** 2026-08-21
**Status:** Draft — architecture proposal + research, no code landed, opinion piece
on feasibility and sequencing as requested
**Scope:** WebSocket pub/sub (`agentmux-srv/src/server/websocket.rs`), cross-channel
discovery/forwarding (`backend/reactive/registry.rs`), jekt trust model
(`backend/reactive/{handler.rs,sanitize.rs}`, `agentmux_common::jekt_sign`)
**Related:** `SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md`,
`SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md`,
`SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`, `SPEC_JEKT_SENSITIVE_TIER_NARROWING_2026_08_15.md`,
`SPEC_JEKT_REAGENT_TRUST_RELAXATION_2026_08_14.md`, `specs/SPEC_CROSS_WINDOW_TAB_REMOUNT_2026_07_11.md`
(**note the path** — this repo has two spec trees, `docs/specs/` and a
top-level `specs/`; this doc lives in the latter. §2.2 there: `Command::MoveTab`
reducer — "blocks and layout state are children of the Tab object — they
travel automatically" — is the real, verified basis for this spec's
single-owner-block invariant. **Correction history, for the record:** reagent's
first review of PR #2721 claimed this file didn't exist; I re-checked with a
`head`-truncated `find` and wrongly agreed, replacing a valid citation with a
false "does not exist" claim in the previous commit; reagent's *second*
review caught that error and pointed at the real path — restored here,
correctly cited this time),
`SPEC_DATA_CHANNELS_2026_05_24.md` (defines "channel" — see naming note below),
`SPEC_AGENT_NAMING_AND_ADDRESSING_HOST_LAN_WAN_2026_08_22.md` (the `HostLabel`
primitive this spec's discovery/presence UI should resolve peers to, §4.3/§5
below — written after this spec, to be consumed by it rather than duplicated)

> **Naming collision to resolve up front.** This codebase already uses "channel"
> for a **data-isolation bucket** (`stable`, `beta`, `local-<branch>-<hash>` — each
> its own srv process, own port, own data dir on the *same* machine —
> `SPEC_DATA_CHANNELS_2026_05_24.md`) — completely unrelated to the Slack/Discord
> sense used by the messaging-bridge specs. **This spec's "cross-channel" means the
> data-isolation sense**: syncing a pane between, e.g., a `stable` install and a
> `local-<branch>-<hash>` dev build running side by side on one host. It has
> nothing to do with the chat-platform bridges. I'll use **"mirror"** (not "sync")
> for the pane-replication feature itself, reserving "sync" for the general
> concept, because "sync" already means something narrower and different in
> collaborative-editing literature (conflict resolution between diverging copies)
> than what's needed here (one live source, N read replicas + floor-controlled
> write access — see §6).

---

## 1. Problem / TL;DR

Today, an agent pane has **exactly one owner** — one window/tab/block, full stop.
Moving it between windows *reparents* it (`Command::MoveTab`,
`specs/SPEC_CROSS_WINDOW_TAB_REMOUNT_2026_07_11.md` §2.2 — "blocks and layout
state are children of the Tab object — they travel automatically"); it never
has two live viewers. The ask: open the same agent ("AgentX") from
a second AgentMux **channel** instance on the same host and have it **mirror
live** — type in one, see it in both, and vice versa — then extend that to LAN and
eventually WAN peers. This is a genuinely new capability, not a variant of
anything that exists. This spec is (a) an inventory of what's actually reusable,
(b) a recommended architecture phased by network scope (same-host → LAN → WAN),
and (c) an explicit accounting of why the security bar rises sharply at each
phase, grounded in this repo's own jekt trust model rather than inventing a
parallel one.

**My opinion, stated up front:** build it in the phased order below, and treat
Phase C (WAN) as a materially different, much higher-risk feature from Phases A/B
— not a scope extension of the same thing. The same-host and LAN cases reuse
infrastructure this repo already trusts (localhost-only forwarding, per-agent Ed25519
LAN signing); WAN mirroring is the first feature in this codebase that would let a
**remote, network-only-proven identity inject keystrokes into a running agent**,
which is precisely the class of action the entire jekt trust layer was built to
require the highest bar for. That doesn't mean don't build it — it means WAN
mirroring needs its own explicit pairing ceremony and default-deny posture, not
"the same feature, just also over the internet."

## 2. Current architecture (code-verified)

**Within one srv instance: real-time pub/sub exists, but it's local and
single-owner.** One `/ws` route per srv instance (`server/mod.rs:391`,
`server/websocket.rs`) is the transport every window/tab of *that instance* uses.
Block/pane content is already published on a per-block scope string
(`"blockstats:block:<id>"`, `websocket.rs:303`) — this is the primitive that lets
a torn-off tab in its own OS window render the same live block: it's really just
another subscriber to one instance's own event bus. **What doesn't exist:** any
second srv instance subscribing to another instance's `block:<id>` stream, or a
block having more than one owning window/tab at a time.
`specs/SPEC_CROSS_WINDOW_TAB_REMOUNT_2026_07_11.md` §2.2 confirms this is
architecturally load-bearing, not incidental: `Command::MoveTab`
(`agentmux-srv/src/reducer/tab.rs:341-449`) reparents `tab.workspace_id` and
"nothing about a tab move touches `blockids` or `layoutstate`" — blocks and
layout are children of the Tab object and travel with it whole, never split
or duplicated across two tabs.

**Correction history, for the record (this went through two rounds of
reviewer back-and-forth and is worth being honest about):** an earlier draft
of this spec cited this exact file and section correctly. reagent's first
review of PR #2721 claimed the file didn't exist. I re-checked with a
`find` command whose output I truncated with `head -20` — the file was
genuinely present, just past the truncation point in a 29-result list — and
wrongly concluded reagent was right, replacing a valid citation with a false
"does not exist" claim. reagent's second review caught that error and
supplied the real path (`specs/`, not `docs/specs/` — this repo has two
spec trees). Restored correctly here.

**`FleetBroadcast` is not a stream — don't build on the name.**
(`agentmux-mcp/src/main.rs:225-257`, handler `main.rs:1373`,
`agentmux-srv/src/server/app_api/fleet.rs` — corrected path, flagged by
reagent's review; the file is not at `backend/app_api/fleet.rs`)
Per its own doc comment: it loops the *same signed single-target `SendMessage`
delivery path*, once per target, client-side. It's "send the same discrete
message N times," not "replicate live state to N viewers." A real terminology
trap — this spec's mirror feature needs new plumbing, not a rename of this tool.

**Cross-channel discrete messaging already exists and is the closest analog.**
`backend/reactive/registry.rs:487-506` (`list_all_shared`) backs `host.cross_channel[]`
in the `/agentmux/discovery` endpoint — a host-global filesystem registry letting
agents in *different* channels on the *same machine* find and jekt each other over
127.0.0.1-only forwarding. Exercised for real on 2026-08-21
(`docs/retro/RETRO_JEKT_CROSS_CHANNEL_TRUST_SELF_DECLARED_2026_08_21.md` — two
agents in different channels messaged each other and it correctly surfaced under
`host.cross_channel` vs. same-channel `host.addressable`). This registry/forwarding
mechanism (`registry/paths.rs`, `resolve_shared_reactive_dir`) is the right
foundation to **discover** a mirror target across channels — it is not itself a
streaming transport, but it's how a mirror request would find the other instance
in the first place.

**UI-capture tools are self-pane-only automation primitives, not mirroring
building blocks.** `UIScreenshot`/`UIQuery`/`UIClick` are explicitly scoped to
"your own pane and shared app chrome... cannot reach a different pane or agent's
UI" (`main.rs:194-223`). `GetAgentTranscript` (`main.rs:133-144`) IS cross-agent
but is a pull-based, best-effort, read-only tail read for Warden/Supervisor
polling — "does not deliver anything to the target." None of these imply or
enable multi-viewer mirroring; `GetAgentTranscript`'s poll-the-transcript-file
pattern is a reasonable *fallback* sync mechanism (§5.4) but is explicitly the
weaker alternative to a live socket, not evidence one exists.

**The jekt trust model — this is the part any mirror-input path must sit inside,
verbatim, not a simplified version of it.** Summarized from this repo's own
`CLAUDE.md` (authoritative, code-anchored):

| `DELIVERY` | Proof mechanism | Outcomes |
|---|---|---|
| `host` | Per-agent HMAC-SHA256 (`AGENTMUX_JEKT_KEY`, injected at spawn, never shared) | `host-verified` (proven), `unverified` (active forgery — always `sensitive`), `self-declared` (no key exists — not "trusted," just unauthenticated-by-construction) |
| `lan` | Per-agent Ed25519 keypair, pubkey fetched from the LAN peer hosting that agent (`SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`) | `lan-verified` (proven, own distinct label), a *found-but-failed* signature is always forced `sensitive` (forged-identity case), unsigned/key-not-found defaults to `network-claimed` (no longer auto-sensitive as of 2026-08-15) |
| `wan` | Only reagent's own service has a pinned key today; general agent-to-agent WAN signing **does not exist yet** (issue #2586's unbuilt half) | reagent `SIG=verified` ≈ `host-verified`; `SIG=invalid` always forced `sensitive`; any other WAN sender is `network-claimed`, exactly as forgeable as before |

**The load-bearing default:** crossing a network boundary **never proves identity
by itself** — `TRUST=network-claimed` is the default for LAN/WAN absent one of the
two narrow cryptographic exceptions above, and clean-content traffic at that trust
level now settles at the *declared* tier (default `coord`) rather than being
auto-escalated — but `sensitive`-forcing red flags (an active signature failure,
a declared-sensitive tier, credential keywords) still apply on top regardless of
trust level, and `ESCALATE=required` on those cases explicitly **cannot be
satisfied by another agent's confirmation over the same network** — only a human,
in-pane. This last rule exists because of a real incident pattern (a spoofed
request followed by a spoofed "confirmation" over the same channel) that maps
directly onto the risk a naive pane-mirror-input feature would recreate if it let
remote keystrokes act with implicit trust.

## 3. Design goals / non-goals

| # | Goal |
|---|---|
| G1 | Output (the agent's own conversation stream) mirrors to every connected viewer in real time, same-host case first |
| G2 | Input (what a human types) from any connected viewer reaches the one underlying agent process |
| G3 | Never weaken the jekt trust model to make this easier — extend it, don't bypass it |
| G4 | Never break the single-owner-block invariant that crash recovery and transcript persistence depend on (`specs/SPEC_CROSS_WINDOW_TAB_REMOUNT_2026_07_11.md` §2.2) |
| G5 | LAN and WAN are explicit, later, opt-in phases — never silently enabled by turning on same-host mirroring |
| NG1 | (non-goal, v1) True concurrent multi-cursor editing of the same in-flight composer text — see §6.2 |
| NG2 | (non-goal) Any of this replacing jekt for actual agent-to-agent coordination — mirroring is human-viewer-facing, not agent-to-agent |

## 4. What a mirror actually needs — splitting read from write

The two directions of "mirror" have very different difficulty, and conflating
them is the most common mistake in this space (confirmed by the collaborative-
editing research below): **output is trivial fan-out; input is the hard part.**

### 4.1 Output (read) — append-only stream, already the easy case

The agent's own transcript is append-only from one source (the agent process).
Mirroring it to N viewers is exactly a pub/sub fan-out problem, not a merge
problem — no CRDT/OT needed. This is close to what `block:<id>`'s existing
WebSocket scope already does *within* one instance; extending it *across*
instances is a transport problem (§5), not a data-structure problem.

### 4.2 Input (write) — this is where real design judgment is needed

If two humans can type into the same composer from two mirrored viewers, what
happens on genuine concurrent edits? Research on this exact tradeoff (collaborative
text editors) is directly applicable and gives a clear steer:

- **CRDTs (Yjs, Automerge) over a WebSocket relay is the modern default** for
  structured collaborative state, and is easier to get right than OT for new
  systems, particularly for offline-tolerant/peer-adjacent scenarios.
- **But CRDTs have real, documented sharp edges for anything beyond plain text**
  — Figma moved *to* CRDTs, but rich-text specifically is flagged industry-wide as
  the case where CRDTs "lose sight of user intent" (e.g. applying formatting near
  a collaborator's cursor can misbehave); Notion's answer was a **hybrid** — CRDT
  for structure, OT for the text *inside* one block, specifically to keep
  fine-grained control where it matters.
- An agent pane's composer is a **single plain-text input**, not a rich document —
  the good news is this sidesteps the hard CRDT cases entirely. But it's also a
  case where **true concurrent typing by two humans is rarely what anyone actually
  wants** (garbled interleaved text, unclear whose "send" wins) — unlike a shared
  document, a chat-style composer's realistic use case is "hand off who's driving,"
  not "two people typing the same sentence."

**Decision for v1: floor control, not CRDT merge.** Exactly one viewer is the
active "driver" at a time (indicated with presence — "Bob is typing" style, easily
derived from the same primitive the research calls "awareness"); switching driver
is an explicit, visible action, not implicit. A CRDT-backed shared composer is
listed as a possible v2 if genuine simultaneous co-typing turns out to be wanted —
but it should not gate v1, and the plain-text nature of this specific composer
means the risk CRDTs are usually chosen to solve (rich-text merge conflicts)
doesn't actually apply here. **"Send"** (submitting a full message to the agent
process) is always a single atomic event regardless of driver model — this is
the one place a simple last-writer-wins-on-submit is sufficient, since only one
submitted message reaches the agent at a time by construction (it's a turn-based
conversation, not simultaneous free-text authorship of one shared document).

### 4.3 Naming — a mirror never mints a new name

Per `SPEC_AGENT_NAMING_AND_ADDRESSING_HOST_LAN_WAN_2026_08_22.md` §4.5: a mirror
connection is a new **viewer** of an existing agent's name, never a new
registry entry and never a name variant. The mirrored pane's chrome renders
the existing (possibly already-qualified) name plus a **presence list** of
active viewers, each labeled with its own `HostLabel` — "AgentX #4 — also open
from: korp-laptop (LAN)." This matters for this spec specifically because it
means the discovery mechanism in each phase below (§5) has exactly one job:
resolve a peer to a `HostLabel`, not mint or reconcile any name.

## 5. Proposed architecture — phased strictly by network scope

### Phase A — same host, cross-channel (lowest risk)

- **Discovery:** reuse the existing cross-channel registry (`registry/paths.rs`,
  `list_all_shared`) unchanged — a mirror target is just another entry alongside
  `host.cross_channel`'s existing agent-discovery use.
  Transport is still 127.0.0.1-only, the same trust boundary this registry already
  operates inside. No `HostLabel` needed at this tier (per the naming spec's
  §4.1, host-tier names are never qualified) — presence indicators, if shown at
  all for same-host viewers, can just say "also open on: \<channel name\>."
- **Transport:** extend the existing per-block WebSocket scope
  (`"blockstats:block:<id>"`) with a new **stream** delivery type that a *different
  srv instance* can subscribe to over localhost, rather than only same-instance
  browser clients. This is new plumbing (no srv-to-srv subscription exists today)
  but stays inside a trust boundary (same machine, same user) this codebase
  already relies on elsewhere (the cross-channel jekt registry makes the identical
  assumption).
- **Authorization:** since this never leaves the machine, treat it like
  `DELIVERY=host` — the mirroring instance and the source instance can mutually
  authenticate the same way an agent's own HMAC key proves host-tier identity
  (§2), just applied to the *srv processes* rather than individual agents.
- **Floor control (§4.2):** whichever viewer's keystroke reaches the srv first
  becomes the driver until idle timeout or explicit hand-off.

### Phase B — LAN

- **Discovery:** reuse whatever LAN peer discovery already backs
  `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` (peers already need to find each
  other to fetch pubkeys for LAN jekt verification — the same discovery answers
  "which LAN peer is running the pane I want to mirror"). Surface that
  resolution as a `HostLabel` (`SPEC_AGENT_NAMING_AND_ADDRESSING_HOST_LAN_WAN_2026_08_22.md`
  §4.2) rather than inventing a second, parallel "which peer" concept just for
  mirroring — the presence indicator (§4.3 above) and the pairing UX below both
  key off the same value.
- **Authorization — this is the part that must not be shortcut:** treat a
  mirror-viewer connection exactly like a `DELIVERY=lan` jekt sender. Extend the
  existing **per-agent Ed25519 LAN signing scheme** to **per-viewer pairing keys**:
  opening a mirror from a LAN peer requires an explicit pairing step (analogous to
  how `tmate`/`upterm` hand out a one-time session address, or how `sshx` issues a
  join link) that mints a keypair the *source* pane's owner explicitly approved —
  not silent, not automatic discovery-based connection. A LAN mirror-input event
  with a signature that fails verification against a known pairing key is a
  forged-identity case and must be treated with the same unconditional-`sensitive`
  severity as a failed LAN jekt signature (§2's table) — worse than unpaired, never
  a lesser version of it.
- **Unpaired/unsigned LAN mirror requests:** default-deny for *write* access
  (input) always; *read-only* mirroring (view but don't type) for an unpaired
  viewer is a reasonable lower-risk option to consider, but even that should
  require the source pane owner's one-time approval, not silent LAN auto-discovery
  — LAN mirroring is meaningfully more sensitive than LAN jekt (a message vs.
  live control of a running agent).

### Phase C — WAN (materially higher risk — treat as a separate feature)

- **Transport: relay server, not P2P WebRTC mesh, for v1.** The collaborative-
  editing research is consistent on this: WebSocket-relay architectures are
  simpler to secure, authenticate, and scale (auth at the relay, filter every
  broadcast by current permission, horizontal scaling via a broker) than raw P2P;
  WebRTC's main advantage (no server in the data path) matters most for
  bandwidth/latency-sensitive peer-to-peer media, not for a text/event stream
  where centralized auth is exactly what you want. Precedent: `upterm` supports
  SSH *and* a WebSocket fallback through a relay (`uptermd`) specifically because
  raw P2P isn't always reachable through NAT/firewalls anyway; `sshx` (the current
  best-of-breed in this space) is fully relay-based and adds real-time
  cursors/chat/presence on top — i.e. the ergonomics users actually want here
  (see reference list) are relay-native, not P2P-native.
- **No general WAN agent-to-agent signing scheme exists yet in this codebase**
  (§2 — issue #2586's unbuilt half). Phase C should **not** invent one just for
  mirroring; it should either wait on that general scheme landing, or (if it ships
  first) use a **narrower, mirror-specific pairing token** modeled on `tmate`/
  `upterm`'s ephemeral session tokens: short-lived, single-use, generated by an
  explicit "share this pane" action on the source side, never a persistent
  standing credential.
- **Mandatory end-to-end encryption of relayed content.** A hosted relay
  otherwise sees every mirrored keystroke and the full transcript stream — which
  may contain exactly the kind of secrets/credentials this repo's jekt keyword-
  scanning already treats as sensitive. The research explicitly flags this as a
  standard pattern (client-side encrypt before the relay, decrypt on arrival) —
  adopt it as a hard requirement for Phase C, not an optional hardening pass.
- **Authorization posture:** WAN mirror-input is `DELIVERY=wan`/`TRUST=network-claimed`
  by default, same as any other WAN traffic in this codebase, **unless** signed
  with a paired token per above. The first connection of a new WAN viewer to a
  pane should carry the **same human-in-the-loop friction as `ESCALATE=required`**
  — explicit confirmation from the pane owner, visible in-pane, not satisfiable by
  any remote party's own claim — precisely because the failure mode being guarded
  against (a spoofed remote actor typing into a running agent) is a strict escalation
  of the exact spoofed-jekt-plus-spoofed-confirmation attack pattern this repo's
  trust layer was hardened against once already (the PR #2536 incident referenced
  in `CLAUDE.md`).

## 6. Decision tables

### 6.1 Transport per phase

| Phase | Transport | Reuses | New work |
|---|---|---|---|
| A (same host) | Extended WebSocket scope, srv-to-srv, localhost | Cross-channel registry, `block:<id>` pubsub shape | srv-to-srv subscription (doesn't exist today) |
| B (LAN) | Same extended scope, over the LAN discovery path | LAN peer discovery, Ed25519 per-agent signing scheme | Per-viewer pairing keys; explicit pairing UX |
| C (WAN) | Hosted relay (WebSocket), E2E-encrypted payloads | Relay-architecture best practice (industry-standard, not this repo's own code) | Relay service itself; pairing-token issuance; E2E encryption layer |

**Decision:** no phase uses raw P2P WebRTC. Even LAN, where P2P is most feasible,
gains nothing over extending the existing WebSocket infra, and WAN specifically
benefits from centralized auth/filtering a relay provides.

### 6.2 Input concurrency model

| | **Floor control (v1, recommended)** | CRDT-merged composer (possible v2) |
|---|---|---|
| Complexity | Low — one boolean (who's driving) + presence indicator | Meaningfully higher — a CRDT library, awareness protocol, merge semantics even for "just text" |
| Matches the actual use case | Yes — "hand off," not "co-author a sentence" | Arguably over-built for a single-line/multi-line plain-text composer |
| Precedent | Standard pairing-tool UX (tmate/upterm/sshx all effectively single-active-typer for meaningful input, even though raw PTY byte-interleaving technically allows simultaneous keystrokes) | Google Docs/Figma/Notion-class rich documents, a different problem shape |

**Decision:** floor control for v1 (§4.2); revisit only if user feedback
specifically asks for true simultaneous co-typing.

## 7. Open questions

| # | Question | Default/Recommendation |
|---|---|---|
| 1 | Does a mirrored viewer need its own full pane chrome, or a lighter "observer" rendering? | Full chrome for the active driver; a visually distinct "observing" state for non-driving viewers, reusing existing presence-indicator patterns rather than inventing new UI |
| 2 | What happens to a mirror connection when the source pane closes/the agent process exits? | Mirror connections should degrade to a clear "session ended" state, not hang or silently reconnect to a new, different agent under the same name |
| 3 | Should Phase A (same-host) require any pairing step at all, given it's already inside a trust boundary this codebase relies on elsewhere (cross-channel jekt)? | Lean toward "no explicit pairing needed for same-host, read-only; still require one confirmation click before granting write/input access" — matches G5's "opt-in, not silent" bar even at the lowest-risk phase |
| 4 | Does Phase C's relay service run as AgentMux-hosted infrastructure, or can users self-host it (mirroring `upterm`'s self-hostable `uptermd` model)? | Recommend self-hostable-by-default, consistent with this project's general preference for not introducing mandatory hosted dependencies — needs a real infra decision, flagged here rather than assumed |
| 5 | How does floor control interact with an agent mid-tool-call or awaiting `AskUserQuestion`? | The driver who receives/answers a validation gate should be unambiguous — likely: whichever viewer is current driver at the moment the gate appears is the only one who can answer it, to avoid two viewers racing to answer the same gate differently |

## 8. Non-goals (v1, all phases)

- True concurrent multi-cursor text editing in one composer (§6.2, NG1).
- Agent-to-agent coordination via this mechanism — mirroring is strictly
  human-viewer-facing; agents still coordinate via jekt, unchanged.
- Mirroring across more than 2 network hops of trust (e.g. WAN peer mirroring a
  LAN peer mirroring a host peer) — v1 is source-instance-to-N-viewers, not
  transitive/relayed mirror chains.
- Recording/replay of a mirrored session as a distinct artifact — out of scope,
  though the existing transcript persistence means a mirrored session is no less
  recoverable than any other.

## 9. References

Internal (this repo, cited above): `SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md`,
`SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`, `SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md`,
`specs/SPEC_CROSS_WINDOW_TAB_REMOUNT_2026_07_11.md`, `SPEC_DATA_CHANNELS_2026_05_24.md`,
`docs/retro/RETRO_JEKT_CROSS_CHANNEL_TRUST_SELF_DECLARED_2026_08_21.md`.

External (research for this spec, 2026-08-21):
- Terminal-sharing architecture survey (tmate/upterm/ttyd/sshx) —
  [saashub.com ttyd vs upterm comparison](https://www.saashub.com/compare-ttyd-vs-upterm-secure-terminal-sharing),
  [Upterm project site](https://upterm.dev/)
- CRDT vs. OT and WebSocket-relay-vs-WebRTC best practices for collaborative
  state — [HackerNoon: CRDTs vs Operational Transformation](https://hackernoon.com/crdts-vs-operational-transformation-a-practical-guide-to-real-time-collaboration),
  [Tiny.cloud: OT vs CRDT](https://www.tiny.cloud/blog/real-time-collaboration-ot-vs-crdt/),
  [Fordel Studios: Real-Time Data Sync patterns](https://fordelstudios.com/research/real-time-data-sync-patterns)
