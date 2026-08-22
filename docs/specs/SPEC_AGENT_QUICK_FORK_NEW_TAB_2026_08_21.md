# SPEC: Quick-fork an agent into a new tab (hot clone, full identity)

**Date:** 2026-08-21
**Status:** Draft — architecture proposal, no code landed
**Scope:** `agent.open` (`agentmux-srv/src/server/app_api/agent_open.rs`), `NewTab`/`SetActiveTab`
MCP tools, `AgentDefinition`/`AgentInstance` model, Armory identity binding
**Related:** `SPEC_MULTI_SESSION_AGENT_FORK_2026_06_06.md` (designs
`ForkAgentDefinitionCommand`, not yet built), `SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md`
(in-pane fork bar — the closest existing sibling feature, `--fork-session` validation
gate already passed there), `SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17.md`
(App-API `agent.*` surface incl. `agent.open`/`agent.fork`/`agent.define`),
`SPEC_AGENT_NAMING_AND_ADDRESSING_HOST_LAN_WAN_2026_08_22.md` (the naming
scheme this spec's forked agents follow, §4.5)

> **Naming.** This is a **quick-fork**, not a duplicate of the in-pane **fork bar**
> from `SPEC_AGENT_PANE_FORKS_AND_AUX_PINS`. Both features share one backend
> mechanism (fork the `AgentDefinition`, spawn with `--resume <sid> --fork-session`)
> and differ only in *where the result lands*: the fork bar pushes the new block onto
> the **same pane's** `blockStack`; a quick-fork opens it as a **new tab**, fully
> independent, immediately visible, no bottom-bar row to notice or switch to. "Hot
> clone" describes the user-facing effect (identical context, up and running
> instantly); "quick-fork" is the mechanism name, kept consistent with the
> codebase's existing fork vocabulary rather than inventing a new one.

---

## 1. Problem / TL;DR

A user mid-conversation hits a fork in the road — two directions worth pursuing, or
a tangent worth exploring without losing the current thread — and wants a **second,
fully independent agent** that starts from the exact same context and can be handed
off to (or run in parallel) immediately, in its own tab. Today:

- `NewTab` (MCP tool) opens an **empty** tab — no agent, no context (`agentmux-mcp/src/main.rs:172-181`
  → `agentmux-srv/src/server/service/tab_lifecycle.rs:20-140`, generic `Command::CreateTab`,
  no agent semantics at all).
- The only existing "fork" UX (`SPEC_AGENT_PANE_FORKS_AND_AUX_PINS`) lands the fork **in the
  same pane**, behind a bottom fork-bar row the user has to notice and switch to. It
  is not yet built (Phase 2–3 of that spec).
- `ForkAgentDefinitionCommand`, the RPC that would mint a forked `AgentDefinition`
  (`SPEC_MULTI_SESSION_AGENT_FORK_2026_06_06.md` §5.1), is **designed but not implemented**.

This spec proposes reusing both of those designs, unchanged, and pointing the
result at a **new tab** instead of an in-pane block-stack — plus resolving one
thing neither prior spec settled: what "full identity" means for the clone, and
whether it inherits the parent's bound credentials (Armory/GitHub) or starts clean.

## 2. Current architecture (code-verified)

**Tab creation is generic and agent-blind.** `NewTab` → `POST /api/v1/tab/new` →
`handle_tab_new` (`agentmux-srv/src/server/mod.rs:1492-1515`) → dispatches
`workspace.CreateTab` → reducer applies events, auto-activates. Produces an empty
tab with no panes. `SetActiveTab`/`FocusWindow`/`Layout` are the read/navigate
verbs around it — none spawn an agent.

**Spawning an agent into a pane is a separate, heavier path.** `agent.open`
(`agentmux-srv/src/server/app_api/agent_open.rs`, ~1113 lines) does the real work:
resolves the `AgentDefinition`, computes `routing_id` (the definition's `slug`) and
sets `AGENTMUX_AGENT_ID` to it (lines ~340-351), serializes cwd/CLI-path/args into
`meta["cmd:env"]`, seeds `agent:sessionid` from the shared registry for resume,
guards concurrent opens of the same definition via `AGENT_OPEN_LOCKS` (lines 22-32),
and dispatches `Command::CreateBlock` through the reducer (required for later
tear-off — comment at lines 503-509). **This is the file a quick-fork action needs
to call into, not `NewTab` alone.**

**Identity is a property of the `AgentDefinition`, not the instance.**
`AGENTMUX_AGENT_ID` = the definition's `slug`. Forking the definition (see below)
already mints a new `id` and, per the auto-naming rule in
`SPEC_MULTI_SESSION_AGENT_FORK`, a new `slug`/label ("Senior Dev" → "Senior Dev #2")
— so **a genuinely new routing identity falls out of forking the definition for
free**; no separate identity-minting step is needed for that layer. Jekt's own
per-agent HMAC signing key (`AGENTMUX_JEKT_KEY`, injected at spawn per this repo's
`CLAUDE.md`) is likewise minted at spawn time, not copied from the parent's stored
`cmd:env` — a forked instance gets its own key by construction, not by inheritance.

**Armory/credential identity is a second, separate layer that is NOT resolved by
forking the definition.** `IdentityAccounts`/`IdentityValidate` read from
`db_agent_identity_links` (per-agent binding rows, `storage/agents.rs:79, 286-289`),
resolved at spawn via `Store::resolve_effective_provider_id`
(`agent_open.rs:57-66`). **Neither `SPEC_MULTI_SESSION_AGENT_FORK` nor
`SPEC_AGENT_PANE_FORKS_AND_AUX_PINS` says whether a forked definition copies these
rows, leaves them unbound, or requires explicit re-binding.** This is the open
question this spec resolves (§5).

**Conversation context is file-based and already provably cheap to clone.**
Provider transcripts are JSONL files keyed by session id
(`SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md`); AgentMux's own UI-snapshot zone
is `agent:<definition_id>:current` (`agent_session.rs`), **one zone per definition**
— which is exactly why forking the *definition* (not reusing it) is required: two
live conversations under the same `definition_id` would collide on that zone.
`--resume <parentSid> --fork-session` was **empirically validated** against the
bundled Claude CLI on 2026-06-15 (`SPEC_AGENT_PANE_FORKS_AND_AUX_PINS` §6.4): the
fork inherits full history to the fork point, mints its own session id, the parent
is untouched, and two forks of the same parent get independent ids. Non-Claude
providers without an equivalent flag fall back to "fork = new definition, fresh
start" — the same honest fallback this spec inherits.

**`AgentInstance` already models lineage.** `{ id, definition_id,
parent_instance_id, block_id, session_id, status, identity_id, memory_id,
instance_name, working_directory }` (`storage/agents.rs`) — `parent_instance_id` is
already the fork-tracking field; nothing new needed here.

## 3. Design goals

| # | Goal |
|---|---|
| G1 | One action, one keystroke/click, from any active agent pane — no modal flow for the common case |
| G2 | Result is a **new tab**, visible and focused immediately (not a bottom-bar row requiring a second action to see) |
| G3 | Clone carries full conversation context up to the fork point (Claude: exact; other providers: honest fallback, no silent context loss) |
| G4 | Clone gets a genuinely distinct identity (`AGENTMUX_AGENT_ID`, jekt signing key) — never shares routing or signing identity with the parent |
| G5 | Credential/Armory identity inheritance is an explicit, informed choice, not a silent default either way |
| G6 | Zero change to the existing runtime invariants (one block = one conversation = one transcript zone) — reuse, don't reinvent |

## 4. Proposed design

### 4.1 Mechanism — reuse verbatim, retarget the landing spot

1. **Fork the definition:** `ForkAgentDefinitionCommand(source=activeDefinitionId)`
   (build per `SPEC_MULTI_SESSION_AGENT_FORK` §5.1's already-designed struct) → new
   `AgentDefinition` with `parent_id`, auto `branch_label`, fresh `working_directory`
   (or explicitly inherit the parent's cwd — see open questions), `is_seeded=0`.
2. **Create the `AgentInstance`:** `parent_instance_id = activeInstanceId`,
   `continueSessionId = activeSessionId`.
3. **Open a new tab:** `NewTab` (existing, unchanged) to get an empty tab, or a
   combined call if the App-API is extended to accept a target definition directly
   (see §4.3).
4. **Spawn into it:** `agent.open` with the forked definition, `--resume
   <parentSid> --fork-session` in `cli_args` (already-supported per §6.3 of the
   forks spec — `subprocess.rs`/`persistent.rs` accept these via `cli_args` today).
5. Tab activates on the new block; parent pane/tab is completely untouched.

This is identical machinery to the in-pane fork bar's §6.3 — the only difference
is step 3 targets a new tab via `NewTab`+`agent.open` instead of pushing a blockId
onto the current pane's `blockStack`.

### 4.2 Decision: new tab vs. in-pane fork bar vs. new window

| | **New tab (this spec)** | In-pane fork bar (existing spec) | New OS window |
|---|---|---|---|
| Visibility | Immediate, full tab, no extra click | Requires noticing/switching the bottom bar | Immediate, but heavier (new window chrome, tear-off machinery) |
| Use case fit | "Hand this off, work it in parallel, side by side" | "Explore a tangent, come back to the main thread" | Rare — multi-monitor workflows |
| Backend reuse | 100% — same fork+spawn mechanism | 100% (source of the mechanism) | 100% + tear-off saga |
| New structural concept | None (tabs already exist) | `blockStack` at the layout node (that spec's one new concept) | None (tear-off already exists) |

**Decision: new tab.** It matches the stated use case (a peer to hand off to, not a
background tangent) with zero new structural concepts — tabs, `NewTab`, and
`agent.open` all exist today. A "quick-fork to new window" variant is a trivial
follow-on (swap the target of step 3 for the existing tear-off saga) but is not
needed for v1.

### 4.3 Trigger / UX

- **Primary affordance: right-click the tab → "Quick-fork to new tab."** Mirrors
  a pattern every user already has from browsers ("Duplicate Tab") — discoverable
  without adding persistent chrome, and it puts the action where the object it
  operates on lives (the tab), matching where the result lands (a new tab).
  Deliberately **not** a new icon in the pane itself: `SPEC_AGENT_PANE_FORKS_AND_AUX_PINS`
  already documents ~16 hand-stacked pane surfaces as a problem to fix, not add
  to, and a pane-level fork icon would risk visual confusion with that spec's
  own in-pane fork bar (two different-looking "fork" affordances in one pane,
  landing in two different places).
- **Secondary: a keybinding** for the no-mouse path (G1) — tab-scoped, in the
  same family as other tab-level shortcuts (new tab, close tab), not a
  pane-level binding, for the same reason as above.
- No modal by default (G1) — one click/keystroke does steps 1-5 above with sane
  defaults (§4.4). An optional long-press/right-click-submenu variant opens a
  small confirmation surfacing the identity choice (§5) for users who want to
  decide per-fork rather than rely on the default.
- Auto-generated tab title = the forked definition's auto-generated name (§4.5),
  consistent with existing fork auto-naming.
- Deferred: should the tab strip's own "+ New Tab" button grow a split/dropdown
  (plain new tab vs. fork current) for users who never discover right-click?
  Ship right-click-only for v1; add the split-button only if discoverability
  turns out to be a real problem in practice.

### 4.4 Non-Claude provider fallback

Identical honesty requirement to `SPEC_AGENT_PANE_FORKS_AND_AUX_PINS` §6.4: for a
provider without an equivalent to `--fork-session`, the new tab still opens
immediately (G1/G2 hold) but starts a **fresh conversation** on the forked
definition, with a visible, non-dismissable-by-accident note in the new tab's
first turn ("this provider doesn't support forking mid-conversation — starting
fresh") rather than silently pretending context carried over.

### 4.5 Naming the fork

Per `SPEC_AGENT_NAMING_AND_ADDRESSING_HOST_LAN_WAN_2026_08_22.md` (written
alongside this spec once the naming question came up): the forked definition
keeps the parent's base name and gets a **flat, lineage-wide counter**
("AgentX" → "AgentX #2"), not a nested/dotted scheme reflecting fork depth
("AgentX #2.1") — the tree structure is already tracked structurally via
`parent_instance_id`, so the display name doesn't need to re-encode it.

The counter is **best-effort, not atomic** — the registry's write primitive
(`registry/atomic.rs`) guarantees no corrupted file, not exclusive allocation
of a number, so two forks of the same lineage minted at nearly the same
moment (even across two different channels on the same host — the registry
is host-global per `SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE`) could both compute
the same next number. Mitigation, per the naming spec: pair the number with
a short collision-resistant suffix (reusing the existing workspace-folder
date+letter tag or a few hex characters of the instance UUID), so a rare
collision is cosmetic (`AgentX #4-0822k` vs. `AgentX #4-0822p`) rather than
truly ambiguous. **v1 of quick-fork only ever lands on the local host** (§8),
so no LAN/WAN qualifier is ever shown — that only becomes relevant if a
future capability forks directly onto a remote peer, at which point the
naming spec's tiering (§4.1 there) already covers it without change here.

## 5. Identity: resolving the open question prior specs left unsettled

**Routing/signing identity (`AGENTMUX_AGENT_ID`, jekt key) is never shared** — this
falls out of forking the definition regardless of any choice made here (§2). The
only real decision is **credential identity** (Armory account bindings /
`db_agent_identity_links`):

| Option | Behavior | Risk |
|---|---|---|
| **Inherit (copy rows)** | Forked agent launches with the same bound GitHub PAT/provider account as the parent | Silent credential fan-out — a "quick" action ends up minting a second agent that can act under the same PAT without the user noticing; harder to audit which agent did what against a shared account |
| **Unbound (default, recommended)** | Forked agent starts with no Armory bindings, same as any newly-created definition; falls back to the global, non-identity-bound `provider_auth_dir()` path (`agent_open.rs`), matching what a plain agent spawn already does today | Fork may not be able to push/PR under the same identity as the parent without a manual re-bind step |
| **Explicit choice at fork time** | The long-press/confirmation variant (§4.3) lets the user pick "same identity" or "unbound" per-fork | Adds one decision to the non-default path only — doesn't slow down the common case |

**Decision: unbound by default, explicit opt-in to inherit.** Matches this
codebase's existing default-secure posture elsewhere (e.g. Armory account
isolation defaulting to isolated-by-channel per `SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md`)
and avoids a "quick" action having a non-obvious credential-sharing side effect.
A user who explicitly wants the clone to act as the same GitHub identity opts in
via the confirmation variant, not the default one-click path.

## 6. Phasing

| Phase | Deliverable | Depends on |
|---|---|---|
| **1** | Implement `ForkAgentDefinitionCommand` (currently just designed, per `SPEC_MULTI_SESSION_AGENT_FORK` §5.1) | — |
| **2** | Wire quick-fork action: `NewTab` + forked-definition `agent.open` with `--resume --fork-session`, unbound-identity default | Phase 1 |
| **3** | UX polish: keybinding, `+` affordance, auto tab title, non-Claude fallback messaging | Phase 2 |
| **4** | Long-press/confirmation variant exposing the identity choice (§5) | Phase 2 |

Phases 1-2 alone deliver the full G1-G4 use case; Phases 3-4 are polish and the
opt-in identity path.

## 7. Open questions

| # | Question | Default/Recommendation |
|---|---|---|
| 1 | Does the forked definition inherit the parent's `working_directory`, or start at a repo-root default? | Inherit — the use case is "pick up immediately," a different cwd would break that |
| 2 | Should quick-fork be blocked/warned when the parent has an in-flight tool call or pending `AskUserQuestion`? | Warn, don't block — forking mid-tool-call is legal (the fork session captures history up to the fork point regardless) but the user should know the fork won't see the in-flight call's *result* |
| 3 | Dormant-fork lifecycle for the new tab (keep-alive vs. suspend) | Same as `SPEC_AGENT_PANE_FORKS_AND_AUX_PINS` §6.6 — no reason to diverge; new tabs are "active" by definition on open, this only matters once the user switches away |
| 4 | Should the fork-bar feature (in-pane) and quick-fork (new tab) share one underlying `useForkAgent()` hook/command, or duplicate? | Share — both call the same `ForkAgentDefinitionCommand` + spawn sequence, differing only in the landing-tab step |

## 8. Non-goals (v1)

- Merging two forks back together, or diffing their transcripts — out of scope,
  same as the in-pane fork spec.
- Cross-machine "hot clone" (spawning the fork on a different host) — that's
  `SPEC_AGENT_PANE_CROSS_CHANNEL_LAN_WAN_SYNC_2026_08_21.md`'s territory, a
  different feature (mirroring a *running* pane, not forking a *new* one).
- A picker/modal for choosing which prior message to fork from (mid-scrollback
  fork points) — v1 forks from "now," matching the validated `--fork-session`
  behavior.

## 9. Files this would touch (orientation)

- **New:** `ForkAgentDefinitionCommand` implementation (backend, per
  `SPEC_MULTI_SESSION_AGENT_FORK` §5.1's struct) if not already landed by the time
  this ships.
- **New:** quick-fork action/command (frontend), likely alongside
  `commands/global/btw.ts` if that has landed, or as its own
  `commands/global/quick-fork.ts`.
- **Reuse, no change:** `agent_open.rs` (spawn path), `NewTab`/`SetActiveTab`
  (`agentmux-mcp/src/main.rs`, `agentmux-srv/src/server/mod.rs`), `AgentInstance`
  storage (`storage/agents.rs`), `--resume`/`--fork-session` `cli_args` plumbing
  (`subprocess.rs`/`persistent.rs`).
- **Touch:** identity-binding resolution at spawn (`agent_open.rs:57-66`) to
  respect the unbound-by-default decision (§5) for forked definitions
  specifically, if it doesn't already default that way.
