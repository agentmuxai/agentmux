# SPEC: Quick-fork an agent into a new pane-stack tab (hot clone, full identity)

**Date:** 2026-08-21
**Status:** Implemented (destination corrected 2026-08-22 — see correction notice below)
**Scope:** `AgentViewModel.getBodyContextMenuItems` (`agent-model.ts`), the
pane's own block-stack (`frontend/layout/lib/layoutStack.ts`),
`AgentDefinition`/`AgentInstance` model, Armory identity binding
**Related:** `SPEC_MULTI_SESSION_AGENT_FORK_2026_06_06.md` (designed
`ForkAgentDefinitionCommand` — **now implemented**, see §2's correction),
`SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md`
(**not a sibling feature — this spec is now understood to complete that
spec's own §6.3 "Fork action (`/btw` and the `+` affordance)", see the
correction notice below**), `SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17.md`
(App-API `agent.*` surface incl. `agent.open`/`agent.fork`/`agent.define`),
`SPEC_AGENT_NAMING_AND_ADDRESSING_HOST_LAN_WAN_2026_08_22.md` (the naming
scheme this spec's forked agents follow, §4.5)

> **Correction (2026-08-22, repo-owner-directed).** This spec originally
> proposed landing the fork in a brand-new top-level **window** tab
> (`WorkspaceService.CreateTab`), reasoning (§4.2 below, kept verbatim for the
> record) that this was a deliberately *different* feature from the in-pane
> **fork bar** in `SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md` rather
> than an implementation of it. That reasoning was wrong on two counts,
> caught live during manual testing: (1) it missed that
> `launchAgentDefinition`'s own `targetBlockId` parameter (`agent-model.ts`,
> shipped 2026-07-20, a month before this spec) already existed *specifically*
> for "the fork-tab-strip `+` action" — i.e. the pane-stack destination this
> spec should have targeted from the start; (2) forking is a **per-agent**
> action, and the codebase already has an established per-agent right-click
> surface for exactly that (`AgentViewModel.getBodyContextMenuItems`, home to
> "Agent History") — the window tab-strip's own context menu, which this spec
> used instead, is ambiguous about *which* agent it targets once a tab can
> ever hold more than one pane, and puts a per-agent action on a per-tab
> surface. **Corrected:** the fork now lands as a new sibling block pushed
> onto the SAME pane's own `blockStack` (`pushBlockOntoStack`, the identical
> primitive `open-history-tab.ts`'s "Agent History" entry and the pane tab
> strip's own "+" already use), triggered from the body context menu right
> next to "Agent History" — not a new window tab, not the tab-strip's
> right-click menu. All of §1-§3's problem framing, §5's identity-binding
> decision, §4.4's non-Claude fallback, and §4.5's naming scheme are
> unaffected by this correction — only *where the result lands* and *where
> it's triggered from* changed. See `frontend/app/view/agent/quick-fork.ts`'s
> own doc comment for the implementation-level version of this history.

> **Naming.** "Quick-fork" is the mechanism name, kept consistent with the
> codebase's existing fork vocabulary. As corrected above, it is now the
> same feature as `SPEC_AGENT_PANE_FORKS_AND_AUX_PINS`'s in-pane fork bar's
> §6.3 fork action — not a distinct sibling. "Hot clone" describes the
> user-facing effect (identical context, up and running instantly in a
> sibling tab in the same pane).

---

## 1. Problem / TL;DR

A user mid-conversation hits a fork in the road — two directions worth pursuing, or
a tangent worth exploring without losing the current thread — and wants a **second,
fully independent agent** that starts from the exact same context and can be handed
off to (or run in parallel) immediately, in its own tab. Today:

- `NewTab` (MCP tool) opens an **empty** tab — no agent, no context (`agentmux-mcp/src/main.rs:172-181`
  → `agentmux-srv/src/server/service/tab_lifecycle.rs:20-140`, generic `Command::CreateTab`,
  no agent semantics at all).
- **Corrected below (reagent + Codex review of PR #2721 caught this — an
  earlier draft of this spec had the current state wrong in two ways):**
  `ForkAgentDefinitionCommand` **is already implemented** (`frontend/app/store/rpc-api/agent.ts:331`,
  backend handler in `agentmux-srv/src/server/agent_handlers/template.rs`),
  not "designed but not built" — the codebase has moved on since
  `SPEC_MULTI_SESSION_AGENT_FORK_2026_06_06.md` was written. It's invoked
  from `AgentPicker.tsx`'s fork prompt today, landing as a separate block/pane
  per that flow. **But the real, shipped fork action does not actually carry
  conversation history** — see §2's `handleFork` finding — despite
  `SPEC_AGENT_PANE_FORKS_AND_AUX_PINS`'s `--resume --fork-session` mechanism
  being empirically validated as *possible*. The validated spike was never
  wired into the real UI action. This spec's G3 (full context carryover) is
  therefore **new work**, not a "just reuse it" claim.

This spec proposes reusing the definition-forking half that already works,
fixing the history-carryover half that was validated but never wired up, and
pointing the result at a **new tab** — plus resolving what "full identity"
means for the clone, and whether it inherits the parent's bound credentials
(Armory/GitHub) or starts clean.

## 2. Current architecture (code-verified)

**Tab creation is generic and agent-blind.** `NewTab` → `POST /api/v1/tab/new` →
`handle_tab_new` (`agentmux-srv/src/server/mod.rs:1492-1515`) → dispatches
`workspace.CreateTab` → reducer applies events, auto-activates. Produces an empty
tab with no panes. `SetActiveTab`/`FocusWindow`/`Layout` are the read/navigate
verbs around it — none spawn an agent.

**Spawning an agent into a pane is a separate, heavier path — and the public
`agent.open` App-API surface is NOT the mechanism that actually supports
forking, contrary to an earlier draft of this spec.** `CommandAgentOpenData`
(`agentmux-srv/src/backend/rpc_types/agent.rs:13-19`) — the request shape for
`agent.open` — has exactly five fields: `agent_id`, `tab_id`,
`split_direction`, `split_reference_block_id`, `focus`. **No resume/continuation
field, no fork flag, nowhere to put `--fork-session`.** Codex's review of PR
#2721 caught this directly: "the proposed spawn call cannot carry the
arguments shown here."

**The real spawn mechanism with fork/resume support is frontend-internal:
`launchAgentDefinition`** (`frontend/app/view/agent/agent-model.ts:320`),
called as `launchAgentDefinition(agent, overrides?, targetBlockId?)`. Its
`overrides` (`LaunchOverrides`) carries `continueSessionId` (resume target)
and a separate `forkSession: boolean` flag; `agent-model.ts:426-428` only
pushes `--fork-session` onto `cliArgs` when **both** `overrides.forkSession`
is true **and** `provider.id === "claude"` — every other provider silently
falls back to a fresh conversation (the honest fallback §4.4 already
describes, now grounded in the actual gating condition rather than assumed).
**This is the function a quick-fork action must call — not `agent.open`.**
The public `agent.open` RPC stays exactly as it is; extending it is not
required, since a quick-fork action lives in the frontend and can call
`launchAgentDefinition` directly, the same way the existing fork-prompt flow
already does (see next finding).

**The existing fork-prompt flow (`AgentPicker.tsx`) proves the definition-fork
half works, and proves the history-carryover half is not actually wired up
today.** `handleFork` (`frontend/app/view/agent/components/AgentPicker.tsx:386-413`):
calls `RpcApi.ForkAgentDefinitionCommand({source_id, branch_label})` (real,
implemented — see above), then `launchAgentDefinition(forkedDef, {
instanceName, agentType, environment, accountId, memoryId })`. **Notice what's
missing from that overrides object: no `continueSessionId`, no
`forkSession`.** Compare `handleReattach` a few lines above it in the same
file, which explicitly sets `continueSessionId: row.session_id ?? ""` for a
plain resume. **The shipped "fork" action mints a new definition (config/
instructions/skills clone) and launches it as a completely fresh
conversation — it does not carry conversation history**, despite
`SPEC_AGENT_PANE_FORKS_AND_AUX_PINS` §6.4 having empirically validated that
`--resume <parentSid> --fork-session` works. The validated mechanism exists;
it was never plumbed into the actual UI action. **Fixing this — adding
`continueSessionId: <parent's session id>, forkSession: true` to the
`launchAgentDefinition` call in the fork flow — is real, scoped, still-needed
work this spec depends on, not something to take for granted as "already
built."**

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

**The existing auto-naming counter is scoped to the immediate parent, not
the lineage — confirmed bug, flagged by Codex's review of PR #2721.**
`template.rs`'s fork-name logic (both the actual fork path, ~line 347, and
`ForkAgentDefinitionSuggestCommand`'s preview, ~line 475) filters
`a.parent_id == cmd.source_id` — counting only definitions forked directly
from *this* source, not the whole tree. Forking "AgentX #2" therefore
produces "AgentX #2 #2," not the flat lineage-wide "AgentX #3" this spec's
naming section (§4.5) and `SPEC_AGENT_NAMING_AND_ADDRESSING_HOST_LAN_WAN_2026_08_22.md`
both require. **This needs a real code change** (walk to the lineage root,
count all descendants, not just `template.rs`'s current immediate-parent
filter) — not just a naming convention this spec can assume already holds.

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

### 4.1 Mechanism — corrected against real code (PR #2721 review round)

An earlier draft of this section assumed `agent.open` could carry
resume/fork-session arguments and that the fork mechanism was fully wired
end to end. Neither is true (§2). The real sequence, using the actual
frontend spawn path:

1. **Fork the definition:** `RpcApi.ForkAgentDefinitionCommand({source_id:
   activeDefinitionId, branch_label})` — already implemented and already
   invoked elsewhere (`AgentPicker.tsx`). Requires the lineage-wide counter
   fix (§2) for `branch_label`'s auto-suggestion to match §4.5's naming
   rule.
2. **Open a new tab:** `NewTab` → capture the returned tab id explicitly.
   **Do not omit it** — Codex's review flagged that `agent.open`/
   `launchAgentDefinition` resolving "whichever tab is active at execution
   time" when no target is passed is a real race if the user switches tabs
   between the two awaited calls, leaving the fork spawned in the wrong
   place and the newly created tab empty.
3. **Spawn into that specific tab:** `launchAgentDefinition(forkedDef, {
   continueSessionId: activeSessionId, forkSession: true, ...identity
   overrides per §5 }, newTabId)` — passing the new tab id as the explicit
   third argument, not relying on ambient active-tab state. **This requires
   fixing the fork-flow's `launchAgentDefinition` call to actually set
   `continueSessionId`/`forkSession`** (§2) — today's `handleFork` doesn't,
   so this is new work, not a "just call the existing thing" step.
4. Tab activates on the new block; parent pane/tab is completely untouched.

Non-Claude providers: `forkSession: true` is silently ignored unless
`provider.id === "claude"` (`agent-model.ts`'s own gating condition) — this is
exactly the honest fallback §4.4 describes, now tied to the real code path
rather than assumed.

### 4.2 Decision: new tab vs. in-pane fork bar vs. new window — **superseded, see correction notice**

The table and "Decision: new tab" conclusion below are kept verbatim for the
historical record of what this spec originally argued — **the actual shipped
behavior is the "In-pane fork bar" column**, per the 2026-08-22 correction at
the top of this document. The reasoning under "Backend reuse" (definition-
forking reuse, the history-carryover fix being a shared prerequisite either
way) held up; the "Decision" line's conclusion did not.

| | ~~New tab (this spec)~~ | **In-pane fork bar — what actually shipped** | New OS window |
|---|---|---|---|
| Visibility | Immediate, full tab, no extra click | Immediate, full pane-stack tab, no extra click (same as "New tab" — the "requires noticing the bottom bar" framing below assumed the OLD read-only fork-bar-as-status-list design, not the block-stack tab strip that actually shipped) | Immediate, but heavier (new window chrome, tear-off machinery) |
| Use case fit | "Hand this off, work it in parallel, side by side" | Same use case, same immediacy — the pane-stack tab strip (`PaneTabStrip`) is a first-class, always-visible tab bar, not a background status row | Rare — multi-monitor workflows |
| Backend reuse | Definition-forking: 100%. History carryover: **0% wired today** (§2) — same fix needed regardless of landing spot | Same — this is the shared prerequisite regardless of destination | Same gap + tear-off saga |
| New structural concept | None (tabs already exist) | None — `blockStack` at the layout node already shipped (`SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md`) before this spec was even written | None (tear-off already exists) |

**Decision (corrected): in-pane fork bar / pane-stack tab.** Forking is a
per-agent action; the pane-stack tab strip is the per-agent-pane surface that
already exists for exactly this (it's what "Agent History" already opens
into). The original "new tab" decision incorrectly treated the two as
distinct use cases ("hand off / work in parallel" vs. "explore a tangent") —
in practice a pane-stack tab satisfies both: it's a first-class, always-
visible tab the moment a second one exists (not a background/dormant row),
and switching or closing it doesn't disturb the parent.

### 4.3 Trigger / UX — **corrected 2026-08-22**

- **Primary (and only) affordance: right-click the agent pane's body →
  "Quick-fork" / "Quick-fork (inherit identity)".** `AgentViewModel.getBodyContextMenuItems`
  (`agent-model.ts`) — the same per-agent right-click surface "Agent History"
  already lives on, spliced at the top of the menu by `blockframe.tsx`. This
  replaces the original "right-click the tab" design (§4.2's superseded
  table): a window tab-strip context menu is the wrong surface for a
  per-agent action once a tab can host more than one pane, and forking
  belongs next to the pane's other per-agent action, not off on a different
  chrome layer.
- No modal by default (G1) — one click does the fork with sane defaults
  (§4.4); a second, explicit "Quick-fork (inherit identity)" menu entry
  covers the identity-choice variant (§5) without a confirmation dialog.
- New pane-stack tab's title = the forked definition's auto-generated name
  (§4.5), consistent with existing fork auto-naming.
- No keybinding for v1 (the original spec's `Cmd:Shift:t` binding operated on
  the active WINDOW tab, which no longer matches this action's actual
  target — the focused PANE's own agent — so it was removed rather than
  repointed; a future keybinding should route through
  `getFocusedBlockInStaticTab()` + `getBlockComponentModel(...)` the way
  other pane-scoped bindings do, but that's a deliberate follow-up, not
  shipped here). The original spec's `SPEC_AGENT_PANE_FORKS_AND_AUX_PINS`
  §6.3 `/btw` slash command remains unimplemented and is also a candidate
  follow-up for a keyboard-first trigger.

### 4.4 Non-Claude provider fallback

Identical honesty requirement to `SPEC_AGENT_PANE_FORKS_AND_AUX_PINS` §6.4: for a
provider without an equivalent to `--fork-session`, the new pane-stack tab still
opens immediately (G1/G2 hold) but starts a **fresh conversation** on the forked
definition, with a visible, non-dismissable-by-accident note ("this provider
doesn't support forking mid-conversation — starting fresh") rather than
silently pretending context carried over. Implemented as a persistent pane
banner (`ForkProviderFallbackBanner`), not a synthesized first-turn message —
see that component's own doc comment for why (no seam to push into the
conversation before the pane's stream mounts).

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
truly ambiguous.

**This is not purely a future-proofing concern — it's a live bug today.**
§2 confirms `template.rs`'s existing fork-count logic filters
`parent_id == source_id` (immediate parent only), so forking an *already-forked*
definition currently produces "AgentX #2 #2," not "AgentX #3." Fixing this —
walking to the lineage root and counting all descendants before formatting
the suggested label — is required before this spec's naming requirement
holds even for the plain in-pane fork flow, not just quick-fork.

**v1 of quick-fork only ever lands on the local host** (§8),
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

Revised against the real state confirmed during PR #2721's review (reagent +
Codex) — `ForkAgentDefinitionCommand` already ships; what's actually missing
is narrower and different from the original draft's Phase 1:

| Phase | Deliverable | Depends on |
|---|---|---|
| **1** ✅ | Fix the two confirmed bugs blocking correctness even for the *existing* fork flow: (a) wire `continueSessionId`/`forkSession: true` into the fork path's `launchAgentDefinition` call (§2 — currently missing entirely), (b) fix `template.rs`'s fork-count filter to walk the lineage root instead of the immediate parent (§4.5) — **landed**, also fixed a third bug found while implementing (b): the suggested name was built from the immediate parent's own name, not the lineage root's, so even a correctly-counted fork of a fork produced "AgentX #2 #3" instead of the flat "AgentX #3" | — |
| **2** ✅ (destination corrected 2026-08-22) | Wire the quick-fork action itself. **Originally shipped landing the fork in a brand-new top-level window tab** (`WorkspaceService.CreateTab`) triggered from the window tab-strip's own right-click menu; **corrected per the notice at the top of this document** to land as a new sibling block pushed onto the SOURCE pane's own `blockStack` (`pushBlockOntoStack` — the same primitive `open-history-tab.ts`'s "Agent History" entry and the pane tab strip's own "+" already use), triggered from `AgentViewModel.getBodyContextMenuItems` right next to "Agent History". The launch reuses `this.launchAgentDefinition` directly (the method's own view-model instance, since the trigger now lives inside that same class) via its pre-existing `targetBlockId` override param — no `targetTabId` override needed post-correction, since the destination never leaves the current window tab. | Phase 1 |
| **3** ✅ | The non-Claude fallback note: `quick-fork.ts`'s `quickForkAgent` sets `FORK_NO_HISTORY_FALLBACK_META_KEY` on the new block once launch succeeds when the fork's effective provider doesn't support `--fork-session` and there was a session to lose; `ForkProviderFallbackBanner` (`agent-view.tsx`) reads it, cloned from `AgentDisconnectedBanner`'s no-dismiss-button pattern since there's no seam to push a synthesized message into the new pane's conversation before its own stream mounts. **The `Cmd:Shift:t` keybinding and the tab-strip "+" split-button from the original draft were both removed/dropped as part of the destination correction** — see §4.3. | Phase 2 |
| **4** ✅ | Confirmation variant exposing the identity choice (§5): a second "Quick-fork (inherit identity)" entry in the body context menu alongside the default quick-fork, calling `quickForkAgent(this, {inheritIdentity: true})`. Resolves the SOURCE definition's own bound account via `ListAgentIdentitiesCommand` (`db_agent_identity_links`), filtered to the fork's own canonical effective provider via `lastLinkedAccountId` (`provider-id-aliases.ts`) — not a raw `.find()` (two review rounds on PR #2735 caught, respectively, taking the wrong-provider link and taking the wrong ROW among same-provider links when a canonical + legacy-alias row both exist). | Phase 2 |

Phase 1 is **not optional polish** — without it, quick-fork (and the existing
in-pane fork prompt) both silently produce fresh-start clones with no
conversation history, contradicting this spec's core promise (G3). Phases
1-2 together deliver the full G1-G4 use case; Phases 3-4 are UX polish and
the opt-in identity path.

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

## 9. Files this would touch (orientation, corrected against real code)

- **Fix (Phase 1a):** `frontend/app/view/agent/components/AgentPicker.tsx`'s
  `handleFork` — add `continueSessionId`/`forkSession: true` to its
  `launchAgentDefinition` call. This fixes the *existing* fork-prompt flow's
  missing history carryover, not just quick-fork's.
- **Fix (Phase 1b):** `agentmux-srv/src/server/agent_handlers/template.rs`'s
  fork-count filter (~lines 347, 475) — walk to the lineage root instead of
  filtering on the immediate `source_id`.
- **New (Phase 2):** quick-fork action/command (frontend) calling
  `launchAgentDefinition` directly with an explicit new-tab id, likely
  alongside `commands/global/btw.ts` if that's landed, or its own
  `commands/global/quick-fork.ts`.
- **Reuse, no change:** `ForkAgentDefinitionCommand` (already implemented,
  `template.rs`), `launchAgentDefinition`'s `LaunchOverrides` shape
  (`agent-model.ts` — `continueSessionId`/`forkSession` already exist as
  fields, just unused by the fork flow today), `NewTab`/`SetActiveTab`
  (`agentmux-mcp/src/main.rs`, `agentmux-srv/src/server/mod.rs`),
  `AgentInstance` storage (`storage/agents.rs`).
- **Not touched, contrary to an earlier draft:** `agent.open`
  (`agentmux-srv/src/server/app_api/agent_open.rs`) and its
  `CommandAgentOpenData` request shape — the real fix lives entirely in the
  frontend `launchAgentDefinition` call, not the App-API surface.
- **Touch:** identity-binding resolution at spawn (`agent_open.rs:57-66`) to
  respect the unbound-by-default decision (§5) for forked definitions
  specifically, if it doesn't already default that way.
