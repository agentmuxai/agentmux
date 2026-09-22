# Spec: Promote Agent Stash from a modal to its own pane

**Status:** proposed — nothing in this spec is implemented. Follows PR #3516
(the backpack icon became a real toggle — highlighted while the Stash modal
is open, retracts on a second click), which this spec explicitly reuses:
the trigger mechanism ships first, this spec redirects what it triggers.

## 0. The ask, verbatim

> lets reconsider a bit .. the indicator is still a good start, but the
> greater change is we want stash to be promoted to it's own pane. Split
> along the longer dimension. The new pane is generic with a new type of
> pane tab called \<agent\>'s stash. Build out that gui from the current
> stash overlay. But the overlay is to be decommissioned.

Four decisions embedded in that ask, each addressed below:

1. Stash becomes a real pane (not a modal), created by **splitting** the
   agent pane it was opened from.
2. The split **orientation** is chosen from the originating pane's own
   aspect ratio ("along the longer dimension").
3. The new pane is a **new pane-tab view type**, labeled `"<agent
   name>'s Stash"`.
4. Its content is **AgentStashModal's existing GUI**, ported in, not
   redesigned — and the modal is fully **decommissioned** once the pane
   exists, not left dead-but-declared (§3.7 explains why this deviates
   from this codebase's usual "leave the superseded kind declared" habit).

## 1. Relationship to existing work

### 1.1 PR #3516 — the toggle icon (shipped, this spec's direct precursor)

The Stash (backpack) button in an agent pane's header used to always open
`AgentStashModal` with no visual state. PR #3516 made it a real
`ToggleIconButtonDecl`: highlighted (`.toggle.active`, `iconbutton.scss`)
while the modal is open, a second click closes it — wired via three new
`AgentModel` callbacks (`_openAgentStashModal`, `_closeAgentStashModal`,
`_isAgentStashOpen`), set in `agent-view.tsx`'s `onMount` and read inside
`agent-model.ts`'s `endIconButtons()`, itself wrapped in a `createMemo` in
`blockframe.tsx` so the highlighted state tracks every close path (X
button, Escape, backdrop click), not just the button's own click.

This spec keeps that indicator and that wiring shape — button still reads
"is Stash open for this agent," still toggles it — but retargets what
"open"/"close" mean: instead of `modalLayer.open({kind:"agent-stash",...})`
/`modalLayer.close()`, they become "split off (or focus) the Stash pane" /
"close the Stash pane's block." See §3.4.

### 1.2 Two precedents in this codebase, neither an exact match

**`docs/specs/archive/SPEC_TOOLCHAIN_TRUSTCENTER_PANE_MIGRATION_2026_06_25.md`**
(archived, shipped) is the closer one: an actual modal-to-standalone-pane
migration (Toolchain Manager, Trust Center). Its "Widget Pattern" checklist
is close to a direct recipe for this migration:

1. Register a `ViewModel` (`viewType`, `viewIcon()`, `viewName()`,
   `viewComponent`) in `block-registry.ts`.
2. Port the modal's render body **verbatim**; drop the `<Modal>` wrapper;
   swap `ModalCloseProps` for `ViewComponentProps<T>`.
3. Repoint openers from `openXModal()` to an open-or-focus-pane helper.
4. Drop now-unnecessary modal machinery (singleton-open guard, backdrop
   dismiss) — "panes are naturally non-modal."

But it migrates to a **standalone, singleton, globally-focusable pane**
(`openOrFocusPaneByView("toolchain")`, no relationship to any other pane).
Stash is per-agent and is explicitly asked to be created by **splitting
off** a specific existing pane — a shape this precedent doesn't cover.

**`docs/specs/SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md`**
(shipped; current code lives in `frontend/app/view/agent/open-history-tab.ts`,
not the spec prose) is a same-pane **tab** creation (`pushBlockOntoStack`),
not a sibling-pane split — a different layout primitive entirely. What
transfers from it is not the mechanism but three hard-won lessons, reused
in §3.4/§3.6:

- **Idempotency**: an in-flight-promise dedup keyed by
  `${currentBlockId}|${agentId}` (`open-history-tab.ts:63-77`) so two
  near-simultaneous triggers can't both miss a not-yet-created target and
  double-create it.
- **Stale-node re-check after an await**: the origin pane can close while
  an RPC is in flight; re-resolve fresh afterward and clean up the orphan
  rather than trusting a pre-await reference (`open-history-tab.ts:141-145`).
- **Remount hazard**: switching a pane's active member is itself a
  remount (`layoutNodeModels.ts`'s `activeKeyFor` keys the subtree on
  `${node.id}:${activeBlockId}`) — anything holding uncommitted draft
  state across that needs an explicit preservation story, not an assumed
  "it just stays mounted."

Neither precedent does **dimension-aware split-orientation choice**. No
code anywhere in this repo picks `createBlockSplitHorizontally` vs.
`createBlockSplitVertically` from the target pane's own aspect ratio —
direction is always an explicit, user-chosen `"up"|"down"|"left"|"right"`
today (`pane-actions.ts`'s `handleSplitPane`, keybindings, command
palette). §3.3 is genuinely new design, not a transcription of existing
code.

## 2. What's confirmed vs. assumed (direct code reading, 2026-09-22)

### 2.1 `AgentStashModal`'s six tabs — migration difficulty per tab

| Tab id | Component | Props needed | Modal coupling |
|---|---|---|---|
| `accounts` | `AgentIdentityLinksPanel` | `agentId` | **None.** Already re-parented into a real standalone pane today — see §2.2. |
| `memory` | `AgentNativeMemoryModal` | `agentId`, `agentName`, `workingDirectory`, `onClose` | **Real coupling** — see §3.6. |
| `mcp` | `AgentMcpModal` | `agentId` | None. Already calls `openOrFocusPaneByView("armory")` itself — i.e. it already knows how to reach a sibling pane. |
| `skills` | `AgentSkillsModal` | `agentId` | None. Same shape as `mcp`. |
| `startup` | `AgentStartupModal` | `agentId` | None. |
| `registration` | `AgentRegistrationPanel` | `agentId` | None. |

Five of six tab bodies take only `agentId` and have zero modal-specific
coupling. Only the Memory tab needs real adaptation work (§3.6).

### 2.2 `AgentIdentityLinksPanel` is already proof this migration is cheap

`frontend/app/view/identity/` already wraps this exact panel in a real
pane (`view: "identity"`): a three-file shape —
`identity-pane-model.ts` (ViewModel: `viewType`, `viewIcon`, `viewName`,
reads `agentId` off `block.meta.agentId`), `identity-pane-view.tsx`
(renders `<AgentIdentityLinksPanel agentId={props.model.agentId()} />` and
nothing else), and a barrel `identity-pane.tsx` that wires
`viewComponent` onto the model's prototype via `Object.defineProperty`
(avoids a circular import between the two). This is the literal template
for the new `agent-stash` view type (§3.1) — one of its six tabs is
already living proof the pattern works.

### 2.3 The registries this migration touches

- **View-type → component**: `frontend/app/block/block-registry.ts` — a
  `Map<string, ViewModelClass>`, one `.set("agent-stash", ...)` line, per
  its own header comment ("`block.tsx` never needs to change").
- **Tab label/icon**: `frontend/app/element/pane-tab-model.tsx`'s
  `describePaneTab` — priority order `meta["frame:title"]` →
  `labelOverride` → a registered `PaneTabDescriptor` → last-seen live
  `viewName()` → widget-bar label → `blockViewToName(view)` fallback
  (`blockutil.tsx`). Simplest correct choice for `"<agent name>'s Stash"`:
  write `meta["frame:title"]` at block-creation time (§3.2) — same
  technique `identity-pane-model.ts` already reads from, just written
  instead of only read.
- **Splitting**: `frontend/app/store/block-layout-actions.ts`'s
  `createBlockSplitHorizontally`/`createBlockSplitVertically`. **Naming
  trap, confirmed against the only real call site
  (`pane-actions.ts:64-100`)**: `SplitHorizontal` produces **side-by-side**
  (left/right) siblings; `SplitVertical` produces **stacked** (top/bottom)
  siblings — the inverse of what "horizontal/vertical" reads as in casual
  English. §3.3 states orientation only in terms of the resulting layout
  ("side-by-side" / "stacked") to avoid an implementer wiring the wrong
  one from the names alone.
- **Pane dimensions**: no existing registry answers "how big is this pane
  right now." But `ViewComponentProps<T>.blockRef` (a DOM ref to the
  pane's own outer element) is populated for **every** view type,
  unconditionally, by `blockframe.tsx` — `AgentViewModel` just doesn't
  currently read it. `pane-size-badge.tsx`'s live width×height overlay
  (`getBoundingClientRect()`, border-box, to match `ResizeObserver`) is
  the pattern to copy for reading it at click time.

### 2.4 Backend maturity: split is fine, creation-then-split has a narrow gap

Per `SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md`, `split` is already one
of the reducer's mature, atomic layout `Command`s
(`agentmux-srv/src/reducer.rs`) — this is NOT the kind of two-step,
can-orphan-a-block flow that spec was written to fix for blockStack tab
pushes. What's still two RPCs, not one: `ObjectService.CreateBlock` (make
the block) is separate from the split action that places it, riding the
older "edit local tree, debounced whole-tree push" flow. A narrow window
exists where the created block could be orphaned if the origin pane closes
mid-flight — the same shape `open-history-tab.ts`'s stale-node re-check
(§1.2) already solved once for the tab-push case; §3.4 reuses that
pattern rather than re-deriving it.

### 2.5 Known-stale/at-risk assumptions to flag, not silently rely on

- `AgentViewModel.giveFocus()` is currently a stub (`return false`) per
  `SPEC_PANE_OPEN_FOCUS_ROUTING_2026_09_16.md`'s own audit — newly created
  panes don't reliably receive focus today. Low-stakes for a mostly
  read-only Stash pane, but this spec does not assume "the new pane will
  be focused" without that stub being fixed or worked around (§5).
- `modal-layer.ts` already carries two now-dead-but-declared modal kinds
  (`AgentIdentityRequest`, `AgentMemoryRequest`, both doc-commented
  "superseded... kept for any future direct callers") from the PRIOR
  consolidation into `AgentStashModal`. This codebase has a habit of not
  deleting a superseded kind. §3.7 explicitly breaks that habit for
  `agent-stash`, per the ask's own "decommissioned" wording, and says so
  out loud rather than silently deviating.
- The Memory tab's narrow-width tab-compression today targets
  `@container modal-mount` — a container name established by
  `ModalLayer.tsx`'s mount node. No generic "pane body" container-type
  exists anywhere under `frontend/app/block` today. §3.6 flags this as new
  design surface, not a drop-in port.

## 3. Design

### 3.1 New view type: `agent-stash`

Three-file shape, cloned from `identity-pane-model.ts` /
`identity-pane-view.tsx` / `identity-pane.tsx`
(`frontend/app/view/agent/stash-pane/` or similar — exact path TBD at
implementation time):

- **`stash-pane-model.ts`**: `viewType = "agent-stash"`. `agentId`,
  `agentName`, `workingDirectory` read off `block.meta` (written once at
  block-creation time by the split action, §3.4 — mirrors how the modal's
  `AgentStashRequest` carried the same three fields). `viewIcon` →
  `"backpack"` (matches `REPORT_ARMORY_STASH_NAMING_2026_07_27.md`'s
  choice — deliberately not `"vault"`, to avoid the exact Armory/Stash
  visual collision that report already solved once). `viewName` reads
  `meta["frame:title"]` with a `"Stash"` fallback (mirrors
  `identity-pane-model.ts`'s `viewName` pattern exactly).
- **`stash-pane-view.tsx`**: renders the ported tab-strip + tab-panel body
  (§3.6) — `AgentStashModal.tsx`'s existing render logic (tabs array,
  active-tab state, panel switch), with the `<Modal>`/`ModalCloseProps`
  wrapper removed and `ViewComponentProps<T>` accepted instead, per the
  Toolchain/TrustCenter migration's own instruction to port "verbatim."
- **`stash-pane.tsx`**: barrel, wires `viewComponent` onto the model's
  prototype via `Object.defineProperty`, same as `identity-pane.tsx`.
- One line in `block-registry.ts`: `blockViewRegistry.set("agent-stash",
  StashPaneViewModel)`.
- One fallback-icon entry in `blockutil.tsx`'s `blockViewToIcon` switch
  (`"agent-stash"` → `"backpack"`) for the case nothing else resolves an
  icon — mirrors the existing `"agent"` → `"sparkles"` entry.

### 3.2 Tab label: `"<agent name>'s Stash"`

Written to `meta["frame:title"]` at block-creation time (§3.4), computed
from the SAME `agentName()` the toggle button and the old modal already
resolve (`agent-view.tsx`'s `agentName()` accessor) — e.g. `"Posa's
Stash"`. Highest-priority tier in `describePaneTab`'s resolution order
(§2.3), so it's static and never needs the block's own `viewName()` to be
reactive. If the agent is later renamed, the tab label does NOT
retroactively update under this design (§6 open question) — matches how
`identity-pane-model.ts`'s `viewName` fallback already behaves for a
missing `frame:title`, not a new inconsistency this spec introduces.

### 3.3 Split orientation — "along the longer dimension"

At the moment the toggle button's `_set` fires with "currently closed,
turn on" (§3.4), before creating anything:

1. Read the origin pane's own outer rect via its `blockRef`
   (`ViewComponentProps<T>.blockRef.current.getBoundingClientRect()`,
   already populated unconditionally by `blockframe.tsx` for every view
   type — `AgentViewModel` just needs to start reading it, no new
   plumbing) — same border-box measurement `pane-size-badge.tsx` already
   uses for its live width×height overlay, for consistency with what the
   user visually sees while resizing.
2. **Wider than tall** (`rect.width >= rect.height`) → the new pane goes
   **side-by-side** → call `createBlockSplitHorizontally(stashBlockDef,
   originBlockId, "after")`.
3. **Taller than wide** (`rect.width < rect.height`) → the new pane goes
   **stacked** → call `createBlockSplitVertically(stashBlockDef,
   originBlockId, "after")`.

`"after"` (not `"before"`) so the Stash pane consistently lands to the
right/below the pane it was opened from, matching the "+"-tab and
mic-target conventions elsewhere in the agent pane chrome (nothing
existing was found that puts a newly-created sibling BEFORE its trigger).

This is genuinely new logic (§1.2/§2.3 — no code anywhere in this repo
picks orientation from pane dimensions today) and needs its own tests
(§4) rather than an assumption it behaves like the existing directional
split call sites.

### 3.4 Open-or-focus / close semantics (replaces `_openAgentStashModal`/`_closeAgentStashModal`/`_isAgentStashOpen`)

Same three-callback shape PR #3516 already wired (§1.1), same call sites
in `agent-model.ts`/`agent-view.tsx` — only what each callback DOES
changes:

- **`_isAgentStashOpen`**: instead of `modalLayer.current()?.kind ===
  "agent-stash"`, scans the origin pane's tab's layout tree for an
  existing sibling leaf whose `meta.view === "agent-stash" &&
  meta.agentId === thisAgentId`. Mirrors `open-history-tab.ts`'s
  `existing` check (§1.2), just querying the layout tree instead of a
  blockStack array — layout-tree lookup already has a working precedent
  in `openOrFocusPaneByView` (`block-component-registry.ts`), adapted here
  to be per-agent rather than global-singleton.
- **`_openAgentStashModal`** (rename to `_openOrFocusAgentStashPane` at
  implementation time): if the scan above finds an existing pane,
  `layoutModel.focusNode(...)` it (no new split). Otherwise: measure
  (§3.3), split, `ObjectService.CreateBlock` with `meta = {view:
  "agent-stash", agentId, agentName: agentName(), workingDirectory,
  "frame:title": \`${agentName()}'s Stash\`}\`, then re-verify the origin
  block still exists before finishing (the stale-node re-check from
  §1.2/§2.4 — the origin pane can close during the `CreateBlock` await).
  In-flight-promise dedup keyed by `${originBlockId}|${agentId}`
  (§1.2) guards two near-simultaneous clicks from double-splitting.
- **`_closeAgentStashModal`** (rename to `_closeAgentStashPane`): finds
  the same sibling leaf and removes its block — i.e. the second click
  genuinely retracts the pane (deletes it), not merely defocuses it. See
  §6 for the alternative (defocus-only, leave it running in the
  background) as an explicit open question, since it changes what happens
  to any in-progress state inside the pane (§3.7).

### 3.5 Focus after creation

Given `AgentViewModel.giveFocus()`'s known stub state (§2.5), this spec
does not assume the new Stash pane receives keyboard/caret focus
automatically. At minimum, `layoutModel.focusNode(...)` (pane-level
selection, distinct from element-level focus) should target the new pane
so it's visibly "the active one" — real caret/keyboard focus inside it is
tracked as an open question (§6), not silently assumed solved by this
migration.

### 3.6 The Memory tab — the one real component-migration task

`AgentNativeMemoryModal` (the `memory` tab's body) is the only one of the
six tabs with real modal coupling (§2.1):

- Its `onClose` prop drives its own footer "Close" button. In a pane,
  there is nothing meaningful to "close" from inside a tab body — the
  footer button is dropped entirely (the pane itself closes via the
  toggle, §3.4, or ordinary pane-close affordances, not a button inside
  one of six tabs).
- Its fixed 780×520 standalone sizing is ALREADY neutralized once, for
  the modal case (`AgentStashModal.scss:135-142`,
  `.agent-stash-modal-panel .agent-memory-modal { width:100%; height:100%;
  max-width:none; max-height:none; ... }`). The pane version needs the
  same neutralization, applied against the new pane's own stylesheet
  instead of `.agent-stash-modal-panel`.
- Its narrow-width tab-compression today keys off `@container
  modal-mount` (`AgentStashModal.scss:117-122`), a container name that
  only exists because `ModalLayer.tsx` establishes it on its mount node.
  No generic "pane body" `container-type` exists anywhere under
  `frontend/app/block` yet (§2.5) — this is new design surface. Simplest
  option: establish a `container-name` on the new pane's own root
  element (scoped to `stash-pane-view.tsx`, no shared/global container
  infrastructure needed) rather than inventing a repo-wide "every pane is
  a container" convention this migration doesn't otherwise need.

`AgentStashModal.scss`'s OUTER modal-sizing rule (`.modal-panel:has(...)
{ width: min(780px,100%); height:560px; ... }`, with its own doc-commented
history of two earlier failed sizing attempts) is dropped outright, not
ported — a pane fills whatever the split gives it; there is no
"shrink-to-fit" concept for a pane the way there is for a modal.

### 3.7 Decommissioning the modal (deliberate deviation from precedent)

Per the ask's explicit "the overlay is to be decommissioned" — unlike
`modal-layer.ts`'s existing habit of leaving a superseded request kind
declared "for any future direct callers" (`AgentIdentityRequest`,
`AgentMemoryRequest`), this migration removes, not deprecates:

- `AgentStashModal.tsx` + `.scss` — deleted once `stash-pane-view.tsx`'s
  content is confirmed to have absorbed everything (§3.6).
- The `"agent-stash"` `ModalLayerRequest` kind in `modal-layer.ts`, and
  its `BACKDROP_DISMISSIBLE_KINDS` entry in `ModalLayer.tsx` — with it
  goes the entire class of risk `ModalLayer.tsx`'s own comments document
  (PR #2315: an accidental backdrop click silently discarding in-progress
  Memory/MCP/Skills draft-form state). A pane has no backdrop to
  mis-click; this is a strict simplification, not something to
  reimplement in the pane.
- `modal-dispatch.tsx`'s `"agent-stash"` case (title `"Stash"`, the
  `<AgentStashModal .../>` render).

Left alone (out of scope, not touched by this migration): the two ALREADY
-dead `AgentIdentityRequest`/`AgentMemoryRequest` kinds predating
`AgentStashModal` itself — cleaning those up is a separate, unrelated
task this spec does not fold in.

## 4. Testing plan

- **Split-orientation logic** (§3.3): pure-function unit tests — given a
  `{width, height}` rect, asserts side-by-side vs. stacked choice,
  including the equal-dimensions tie-break (`width >= height` → wider
  path, stated explicitly so it's not left as an unspecified `>` vs. `>=`
  edge case).
- **Open-or-focus dedup** (§3.4): mirrors `open-history-tab.ts`'s own test
  coverage shape — two near-simultaneous triggers produce exactly one
  pane; a trigger while a matching pane already exists focuses instead of
  splitting again; the stale-origin-pane-closed-mid-await path cleans up
  the orphan rather than leaving it stranded.
- **Toggle round-trip**: open (creates + focuses) → button reads active →
  close (removes the block) → button reads inactive again — reusing PR
  #3516's existing `_isAgentStashOpen`/toggle wiring tests as the
  template, retargeted at the new pane-based implementations of the three
  callbacks instead of `modalLayer`.
- **Tab content parity**: each of the six ported tab bodies still renders
  and functions with only `agentId` (+ the Memory tab's fuller prop set)
  — five of six need no new test coverage beyond "still mounts," since
  their own component-level tests are unaffected by the wrapper around
  them; the Memory tab needs a regression test confirming removing the
  `onClose` footer button doesn't silently orphan any Save/Cancel flow
  that used to route through it.
- **View-type registration**: `block-registry.ts`'s `getBlockViewClass
  ("agent-stash")` resolves; `describePaneTab` produces `"<name>'s
  Stash"` from `frame:title` before falling through any lower-priority
  tier.

Live verification (manual): open Stash from a wide pane (confirms
side-by-side), from a tall pane (confirms stacked), confirm the tab label,
confirm a second toggle click removes the pane, confirm the old backpack
modal no longer opens anywhere.

## 5. Risks / tradeoffs — stated plainly, not resolved here

- **Focus after split is not guaranteed** (§2.5/§3.5) — `giveFocus()`'s
  stub state is a pre-existing gap this spec inherits rather than fixes.
  If real focus/caret behavior inside the new pane turns out to matter
  (e.g. for the Memory tab's inline-edit textarea), that stub needs
  fixing as a prerequisite, not worked around locally here.
- **"Retract" = delete the block** (§3.4) is a real, mildly destructive
  choice — any in-progress state inside the pane (a half-filled MCP/Skills
  "+New" form, an in-flight Memory edit) is discarded on second-click,
  same as it would be discarding a modal's draft today, but now with a
  slightly higher-stakes "pane" framing that a user might expect to
  persist more than a modal would. §6 asks whether that's actually
  wanted.
- **Split creates real layout state**, unlike a modal: a user who never
  explicitly closes the Stash pane now has a permanent extra pane in
  their tab's layout tree (saved/restored across reloads, unlike a modal
  which never persisted). This is a deliberate consequence of "promote to
  a pane," not a bug, but worth naming since it's a real behavior change
  from today (opening the modal has zero persistent-state footprint;
  opening the pane does).
- **The dimension-aware split is genuinely new code** (§1.2/§3.3) with no
  existing pattern to lean on for correctness — higher implementation
  risk than the rest of this migration, which is largely mechanical
  porting.

## 6. Open questions

1. **Does "retract" mean delete the pane's block, or just defocus it
   (leaving it alive in the background, re-focusable on a third click)?**
   §3.4/§5 assume delete, matching the toggle's own "closes what it
   opened" framing from PR #3516, but this is a real product decision
   with different data-loss implications, not obviously settled by the
   original ask's wording alone.
2. **Should the tab label live-update if the agent is renamed after the
   Stash pane is created** (§3.2), or is the static `frame:title`
   snapshot from creation time acceptable? `identity-pane-model.ts`'s
   existing `viewName` already tolerates a similar staleness for its
   `frame:title` fallback, so following that precedent (static) is the
   lower-effort default — flagged rather than assumed.
3. **Does closing the ORIGIN pane (the agent pane the Stash pane was
   split from) also close its Stash pane**, or does the Stash pane become
   an orphaned sibling with no obvious "which agent is this for" context
   once its origin is gone? Neither existing precedent (§1.2) answers
   this — `open-history-tab.ts`'s tabs live INSIDE the same blockStack as
   their origin and close together by construction; a split sibling has
   no such structural guarantee.
4. **Multiple Stash panes for the same agent, opened from different tabs
   simultaneously** (the agent open in two different pane tabs at once,
   Stash toggled from both) — is the open-or-focus dedup (§3.4) scoped
   per-tab or globally per-agent? The layout-tree scan as described is
   per-tab (mirrors `open-history-tab.ts`'s per-blockStack scope); a
   global per-agent singleton (mirrors `openOrFocusPaneByView`'s scope
   instead) is the alternative and wasn't decided here.

## 7. Out of scope

- Cleaning up the two ALREADY-dead `AgentIdentityRequest`/
  `AgentMemoryRequest` modal-layer kinds predating this spec (§3.7).
- Fixing `AgentViewModel.giveFocus()`'s stub state (§2.5/§5) — tracked as
  a pre-existing gap this spec inherits, not a prerequisite this spec
  commits to resolving.
- A repo-wide "every pane is a CSS container" convention (§3.6) — this
  spec scopes the Memory tab's container-query need to the new pane's own
  root element only.
- Any visual/UX redesign of the six tabs' own content — §3.1/§3.6 are
  explicit about porting the existing GUI, not redesigning it.
