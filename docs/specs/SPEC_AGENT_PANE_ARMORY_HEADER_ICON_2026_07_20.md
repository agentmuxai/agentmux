# SPEC: Vault Icon on the Agent-Setup Button + Responsive Tabs in the Per-Agent "Armory"

**Date:** 2026-07-20 (corrected 2026-07-21, extended 2026-07-21 §7)
**Status:** implemented — PR #2253 including §7 (horizontal-scroll elimination); modal since renamed Stash (#2314). Verified 2026-08-10.
**Scope:** `frontend/app/view/agent/agent-model.ts`,
`frontend/app/view/agent/components/AgentSetupModal.tsx` / `.scss`,
`frontend/app/view/agent/components/AgentNativeMemoryModal.tsx` / `.scss`,
`frontend/app/view/agent/agent-native-memory-model.ts`,
`frontend/app/view/agent/components/AgentIdentityModal.tsx` /
`_identity-panel.scss`

---

## 0. Correction note

The first version of this spec (and its initial implementation) misread the
ask as "add a **new** header icon that opens the **global** Armory pane, and
make the pane **header** itself narrow-width responsive." That was wrong on
both counts:

- There is no new icon and no new "open the global Armory" action. The
  existing "Agent setup" (`id-card`) icon **gets restyled to the vault icon**
  — same button, same click handler, icon only.
- "The armory" in the original ask refers to **`AgentSetupModal`** — the
  tabbed Accounts/Memories/MCP Servers/Skills modal that icon already opens,
  informally called "the agent armory" (a per-agent-scoped analogue of the
  global Armory pane) — not the global Armory pane itself. All the
  thinner-panes discussion was about making *that modal* degrade the way the
  global Armory pane does, not about the pane header.

The pane-header changes from the first pass (a second `endIconButtons` entry,
a `block.scss` container-query + hide-priority system) have been reverted in
full. Nothing in `blockframe.tsx`/`block.scss` is touched by this spec anymore
— see §5 for confirmation of that reduced blast radius.

---

## 1. Ask (corrected)

The existing "Agent setup" icon (top-right of an agent pane, opens
`AgentSetupModal`) should simply **use the vault icon** instead of `id-card` —
same icon Armory uses elsewhere, since this modal is effectively a per-agent
Armory. Separately, `AgentSetupModal` itself should support thinner widths the
way the global Armory pane does (it can genuinely get narrow, since it caps at
`92vw` of the app window, not a fixed size).

---

## 2. Current state (investigated)

### 2.1 The existing button — `agent-model.ts:141-152`

```ts
this.endIconButtons = () => {
    const agentId = this.blockAtom()?.meta?.["agentId"];
    if (!agentId) return [];
    return [
        { elemtype: "iconbutton", icon: "id-card", title: "Agent setup",
          click: () => { this._openAgentSetupModal?.(); } },
    ];
};
```

Hidden until an agent is loaded (empty array on the picker screen). Icons are
rendered through the shared `IconButton` component
(`frontend/app/element/iconbutton.tsx:12`) via the declarative `IconButtonDecl`
type — `icon: "vault"` resolves to `fa fa-solid fa-vault fa-fw` via
`makeIconClass`. Changing the icon is a one-line edit; the click handler,
gating, and everything else about the button stays exactly as-is.

### 2.2 What it opens — `AgentSetupModal.tsx`, "the agent armory"

```
frontend/app/view/agent/components/AgentSetupModal.tsx
```

A modal (opened via the global `useModalLayer()`, not confined to the
originating pane's DOM bounds — `agent-view.tsx:194-218`) with a horizontal
top tab bar and four tabs, each delegating to an existing standalone panel:

| Tab id | Label | Delegates to |
|---|---|---|
| `accounts` | Accounts | `AgentIdentityModalPanel` |
| `memory` | Memories | `AgentNativeMemoryModal` |
| `mcp` | MCP Servers | `AgentMcpModal` |
| `skills` | Skills | `AgentSkillsModal` |

Before this change, the tab bar was **text-only** (`{tab.label}`, no icons at
all), and the modal (`.agent-setup-modal`, `AgentSetupModal.scss:9-20`) had a
fixed `width: 780px; max-width: 92vw;` with **no responsive/container-query
handling whatsoever** — the tab bar just sat there at whatever width the
92vw cap left it, with no adaptation.

### 2.3 The reference pattern — the global Armory pane's rail

`frontend/app/view/armory/armory-view.tsx` / `.scss` — same four concepts
(plus a fifth, Bundles, which has no per-agent equivalent) as a vertical rail
with icon + label:

```ts
const RAIL: { id: ArmorySection; label: string; icon: string }[] = [
    { id: "accounts", label: "Accounts",    icon: "key" },
    { id: "brain",    label: "Memories",    icon: "brain" },
    { id: "skills",   label: "Skills",      icon: "wand-magic-sparkles" },
    { id: "mcp",      label: "MCP Servers", icon: "plug" },
    { id: "memories", label: "Bundles",     icon: "layer-group" },
];
```

Markup per item: `<i class="fa-sharp fa-solid fa-{icon}" aria-hidden="true"
/><span>{label}</span>` — the label lives in its own `<span>` specifically so
CSS can hide just the text and keep the icon.

Responsiveness is pure CSS `@container`, no JS/`ResizeObserver`:
- `.armory-container` (a wrapper `armory-view.tsx:30-31` needs, since a
  container can't query its own width) carries
  `container-type: inline-size; container-name: armory;`
  (`armory-view.scss:9-14`).
- `@container armory (max-width: 767px)` — compress the rail from `168px` to
  `48px`, hide `span` labels, icon-only.
- `@container armory (max-width: 479px)` — swap layouts entirely: hide the
  rail, show a bottom tab bar instead (always in the DOM, toggled by
  `display`).

`AgentSetupModal`'s tab bar is **already** a horizontal row (Armory's *narrow*
fallback shape), so only the label-hiding half of Armory's pattern applies —
there's no rail-to-swap transition needed since it never had a rail to begin
with.

---

## 3. Implementation (done)

### 3.1 Icon swap — `agent-model.ts`

`icon: "id-card"` → `icon: "vault"`. No other change to that button.

### 3.2 Icons + responsive tabs — `AgentSetupModal.tsx` / `.scss`

- `SetupTabDef` gained an `icon: string` field; each tab now carries the same
  icon as its matching Armory-rail concept (`key` / `brain` / `plug` /
  `wand-magic-sparkles`) — visual parity with the pane this modal is the
  per-agent analogue of.
- Tab button markup now matches Armory's exactly: `<i class="fa-sharp
  fa-solid fa-{icon}" aria-hidden="true" /><span>{label}</span>`, plus a
  native `title={label}` on the button (Armory uses a `<Tooltip>` wrapper for
  this; a native `title` attribute was used here instead to avoid pulling in
  that component for one tab bar — same information, simpler dependency
  footprint).
- `.agent-setup-modal` gained
  `container-type: inline-size; container-name: agent-setup;` — it's already
  the right ancestor for `.agent-setup-modal-tab` (a descendant), so unlike
  Armory this needed no extra wrapper `<div>`.
- One breakpoint, `@container agent-setup (max-width: 560px) { .agent-setup-modal-tab
  span { display: none; } }` — hides tab labels, keeps icons, once the modal
  gets narrow (a real scenario given the `92vw` cap on a small window).
  560px, not Armory's 767px: this bar only ever holds 4 short-ish labels
  (vs. Armory's rail holding 5, some longer — "MCP Servers"), and the
  available width per tab in a horizontal bar differs from a vertical rail's
  per-item width, so reusing Armory's number wasn't meaningful here either —
  picked to leave comfortable room for 4 icon-only tabs well before the tab
  bar would visibly wrap or truncate.

---

## 4. Verified

- `npx tsc --noEmit` — clean.
- `npm run lint:scss` — no new errors.
- `npx vitest run frontend/app/view/agent` — 669/670 passing (1 pre-existing,
  unrelated timeout flake under full-suite machine load — confirmed passing
  in isolation). No dedicated test file exists for `AgentSetupModal`.
- Not done: live interactive click-through of the narrow-width collapse.
  `task dev` was launched separately for manual testing rather than scripted
  automation, since no project skill/driver exists for interactively driving
  this native macOS CEF app (not Electron, no existing Playwright-style
  harness), and a competing dev instance risked interfering with the live
  production session this conversation itself runs inside of.

---

## 5. Blast radius (now much smaller than the first pass)

Three files: `agent-model.ts` (one-line icon change), `AgentSetupModal.tsx`
(tab icons + markup), `AgentSetupModal.scss` (container query + icon
styling). `blockframe.tsx` and `block.scss` — shared by every pane view type
— are **untouched**, unlike the reverted first attempt. Nothing outside the
agent-setup surface is affected.

---

## 6. Non-goals

- Not touching the global Armory pane's own code
  (`armory-view.tsx`/`.scss`) — reference pattern only.
- Not touching the existing failure-state "Open Armory" banner
  (`failure-accessory.ts`), which does open the *global* Armory pane — that's
  a genuinely different, unrelated affordance and was never in scope.
- Not adding a rail-to-bottom-bar layout swap to `AgentSetupModal` — its tab
  bar is already the shape Armory swaps *to* at narrow widths, so there's no
  second layout to transition into.

---

## 7. Follow-up: eliminating horizontal scroll (live-tested 2026-07-21)

Asaf tested §1-6 in a real `task dev` session and found horizontal scrolling
on the modal — not isolated to one tab. Direct quote: *"horizontal scrolling
should not appear on any tabs. the whole design needs to be rethought."*
Investigated fully before touching more code, since this task had already
been misread twice.

### 7.1 Root cause — one bug, hits every tab identically

`AgentSetupModal.scss`:

```scss
.agent-setup-modal {
    width: 780px;
    max-width: 92vw;
    height: 560px;
    max-height: 85vh;
    ...
}
```

This modal opens **pane-scoped** (`useModalLayer()` inside a per-agent-pane
component, `agent-view.tsx:194-218`) — its root sits inside the originating
pane's own mount node (`frontend/app/element/ModalLayer.tsx:104-107`,
`.modal-layer-mount`), not the whole browser window. `92vw`/`85vh` are
computed against the **viewport**, not that mount node — so on a narrow pane
inside a wide window, `92vw` evaluates to something far larger than the
pane's real width and constrains nothing. The modal renders at its literal
`780px`/`560px`. `.modal-panel` (`modal.scss:87-97`) correctly clamps itself
to `max-width: 100%` of its real ancestor and has `overflow: auto` — so the
780px child overflows it, and `.modal-panel`'s own scrollbar is exactly the
"horizontal scrolling" reported, present on every tab because they all share
this one outer shell.

**The fix already exists in this codebase, just isn't applied here.**
`modal.scss` documents two layers (comments at lines 106-146, tagged
"MODAL_COMPACT_VARIANT_ARCHITECTURE_2026_05_26"):
- **Layer B** — fluid sizing via `width: min(<target>px, 100%)` instead of a
  raw px/vw combo (already used by `.modal-panel[data-size="sm"|"md"|"lg"|"xl"]`,
  `modal.scss:112-115`). `ModalLayer.tsx:114` hardcodes `size="fit"` for every
  `modalLayer.open()` call, so `.modal-panel` itself is `width: auto; max-width:
  100%` — sizing is the *content's* job for a "fit" modal, and
  `AgentSetupModal.scss` never applied Layer B's technique to itself.
- **Layer C** — a `min-width: 0` cascade for modal body content, gated by
  `@container modal-mount (max-width: 400px)`, but scoped to
  `.modal-panel-body[class]` (`modal.scss:141-146`) — a class `ModalBody`
  (`modal.tsx:626-628`) applies. `AgentSetupModal`'s root div never carries
  this class, so this rescue doesn't reach it either.

**Fix, as actually implemented (revised once — see the live-bug note
below):** change `.agent-setup-modal`'s `width` to `min(780px, 100%)`,
dropping the `max-width: 92vw` — `min()` subsumes it. **Height stays a plain
`height: 560px; max-height: 85vh;`, unchanged from before.** The first
attempt applied the same `min(560px, 100%)` treatment to height, symmetric
with width — this was untested scope creep (the reported bug was
horizontal-only) and broke the modal outright in live testing: stuck on a
blurred backdrop with unrenderable/unreachable content, no way to dismiss
it. Root cause: unlike width, `.modal-panel` never gets an *explicit*
height — only `max-height` (`modal.scss:151-154`'s pane-scoped override caps
it at `100% - 48px` of a real ancestor, but a max is not a definite height).
Per CSS, a percentage height with no definite containing-block height
resolves to `auto`; mixing that into `min(560px, 100%)` is undefined/
inconsistent across engines in practice, and in this codebase's actual CEF
renderer it broke.

**Third correction — width had the same collapse bug, just not caught until
live DOM inspection.** After the height revert shipped, Asaf reported the
modal *still* stuck on a blurred backdrop, unrecoverable, even after a full
process restart (ruling out stale HMR). The claim above — "percentage-width
resolution against a shrink-to-fit ancestor is well-defined and works" — was
wrong. Verified live via Chrome DevTools Protocol against the running CEF
renderer (`--remote-debugging-port`, `Runtime.evaluate` reading
`getComputedStyle()`/`getBoundingClientRect()` on the actual stuck modal):
the full DOM tree rendered correctly (tabs, Accounts panel, provider rows,
all present in `outerHTML`) but `.agent-setup-modal` computed to `width:
0px` and `.modal-panel` to `width: 2px` (just its border) — invisible, not
broken. Root cause: `min(780px, 100%)` on `.agent-setup-modal` is a
percentage on a *child* of `.modal-panel[data-size="fit"]`
(`width: auto; max-width: 100%` — shrink-to-fit, sized *by* its content).
The parent's width depends on the child's preferred size; the child's `%`
depends on the (not-yet-resolved) parent's size — a circular dependency
that Chromium resolves by treating the percentage as ~0, collapsing both
boxes.

A first attempt at fixing this swapped `100%` for `100cqw` (a CSS
container-query length unit, resolving against the nearest
`container-type` ancestor — `.modal-layer-mount`, `container-name:
modal-mount`, `ModalLayer.tsx:104-107` — instead of the immediate parent).
That did stop the collapse (confirmed live: non-zero width), but introduced
a *different* live-confirmed bug: `modal-mount` tracks the full pane, which
is wider than `.modal-panel`'s own shrink-wrapped, backdrop-clamped box, so
the child (595px) ended up wider than its actual parent (547px) —
re-overflowing past `.modal-panel`'s edge from the other direction.

**Final fix:** stop trying to size `.agent-setup-modal` (the child)
independently at all. Instead put the `min(780px, 100%)` sizing on
`.modal-panel` itself, scoped with a `:has()` selector to only apply when
it's hosting this modal (`:has()` is already precedented in this codebase —
`_document.scss:107-120` — and supported by the pinned CEF version, Chromium
105+):

```scss
.modal-panel:has(.agent-setup-modal) {
    width: min(780px, 100%);
    height: 560px;
    max-height: 85vh;
}

.agent-setup-modal {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-width: 0;
    container-type: inline-size;
    container-name: agent-setup;
}
```

This is the exact same technique `modal.scss`'s own `[data-size="sm"|"md"|
"lg"|"xl"]` rules already use — the percentage lives on `.modal-panel`,
resolving against *its* parent `.modal-root` (`position: fixed|absolute;
inset: 0` — genuinely definite, unlike `.modal-panel` itself). The child
then just fills whatever `.modal-panel` resolves to; no circular dependency,
no independent sizing that can disagree with the parent. Re-verified live
via the same CDP DOM-inspection technique across all 5 tabs at the pane's
actual (narrow) width: `.modal-panel` / `.agent-setup-modal` both resolve to
547px / 545px (2px border), `scrollWidth === clientWidth` on every tab
(no horizontal scroll), and the icon-only tab-label breakpoint fires
correctly at that width.

For the `min-width: 0` rescue, rather than adopting the `modal-panel-body`
class (which also carries generic padding/font-size from `modal.scss:217-221`
that don't belong on this component), the same rule was written scoped to
`AgentSetupModal`'s own classes directly, targeting the same `modal-mount`
container `ModalLayer.tsx`'s mount node already establishes:
```scss
@container modal-mount (max-width: 400px) {
    .agent-setup-modal-tabs,
    .agent-setup-modal-panel {
        min-width: 0;
    }
}
```

Same underlying width bug independently affects the still-live standalone
`agent-identity`/`agent-memory` modal-dispatch paths
(`AgentIdentityModal.scss:12-13`, `AgentNativeMemoryModal.scss:13-14`) — out
of scope here (they're superseded by the tabbed modal per
`AgentSetupModal.tsx`'s own doc comment) but flagged for awareness.

### 7.2 Memories tab — fixed 220px list column, zero fallback

`AgentNativeMemoryModal.scss:74-75`: `.agent-memory-modal-list { flex-shrink:
0; width: 220px; }` inside a two-column flex row
(`.agent-memory-modal-body`). Unlike Accounts (§7.3), there's no responsive
fallback at all here — the column is simply pinned, and independently forces
overflow once the modal is much narrower than ~350-400px, even after §7.1's
fix.

MCP Servers and Skills tabs already solved this exact "list + detail, must
work at any width" shape properly, via a shared, already-adopted component:
`PrimitiveListDetail` (`frontend/app/element/primitive-list-detail.tsx`,
from `SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md`) — shows
**exactly one** of {list, detail} at a time, no side-by-side split, no fixed
widths anywhere. Memories never adopted it (predates it, or was just missed).

**Fix: migrate Memories to `PrimitiveListDetail`**, matching
`AgentMcpModal.tsx`'s existing wiring pattern (`showDetail`, `backLabel`,
`onBack`, `list`, `detail` props):
- `showDetail = () => model.selectedFilenameAtom() != null`.
- `list` = the existing file-list + new-file-input + "+ New file" button
  (today's `.agent-memory-modal-list` content), **with the current
  `EmptyState`'s "no files yet" call-to-action (heading + description + "+
  Create MEMORY.md" button) moved into the list's empty branch**, replacing
  the current bare "No files" text — that CTA needs to live somewhere
  reachable now that there's no separate detail pane to show it in.
- `detail` = the existing view/edit content (today's `.agent-memory-modal-detail`
  content), with its `Show ... fallback={<EmptyState .../>}` **removed** —
  under `PrimitiveListDetail`, detail only ever renders when a file *is*
  selected, so the "nothing selected" fallback can't occur there anymore.
- `onBack` clears the selection — needs a new `clearSelection()` method on
  `AgentNativeMemoryModel` (currently only `selectFile(filename: string)`
  exists, no way to set it back to `null`).
- **Behavior change, called out explicitly:** `loadFiles()`
  (`agent-native-memory-model.ts:112-125`) currently auto-selects the first
  file whenever none is selected and files exist, specifically so — per its
  own comment — "the modal never opens to an empty right pane." Under
  single-pane, that rationale no longer applies (there is no separate right
  pane to be empty), and keeping the auto-select would make Memories
  inconsistent with its sibling tabs — MCP Servers/Skills/Startup all open to
  their list, never jump straight into an item's detail. **Removing the
  auto-select** so Memories opens to its list too, matching the other tabs,
  is the more consistent choice — flagged here since it's a real, deliberate
  behavior change, not an incidental side effect of the refactor.

### 7.3 Accounts tab — provider-row grid has a non-shrinking floor

`_identity-panel.scss:38-40`: `.agent-identity-provider-row { display: grid;
grid-template-columns: 16px 72px 1fr auto; }`. The `auto` track holds a
`<select>` (`max-width: 140px`, no min-width) plus a "+ New" button
(`white-space: nowrap`, no ellipsis) plus, when assigned, an unassign "×"
button. Grid `auto` tracks size to their content's min-content and do not
shrink below it regardless of ancestor `min-width: 0` (that fixes flex items;
it doesn't touch a grid track's own intrinsic sizing) — rough floor ≈
370-420px for one row, before the panel's own padding.

**Fix: reflow to two rows per provider below a breakpoint**, rather than
trying to force the actions cell to compress (a `<select>` and two buttons
don't have meaningful room left to give). Add a container-query breakpoint
(reusing `.agent-setup-modal`'s own `agent-setup` container context from §1,
since `AgentIdentityModalPanel` renders as a descendant of it when embedded)
that switches `.agent-identity-provider-row` from
`grid-template-columns: 16px 72px 1fr auto` to `grid-template-columns: 16px
1fr; grid-template-rows: auto auto;`, with the actions cell
(`.agent-identity-provider-assignment` or a wrapping element) spanning the
second row's full width — giving the select + buttons the whole row's width
to work with instead of a squeezed single column. Exact breakpoint: pick
empirically against the ~370-420px floor above (a value comfortably above it,
so the reflow triggers before content actually clips) — recommend starting
around 440-460px and adjusting after a live check, same approach as §3.2's
560px pick.

Also worth a one-line fix regardless of the breakpoint:
`.agent-identity-provider-label` (line 57-60) has `white-space: nowrap`
with no `overflow: hidden; text-overflow: ellipsis` pair — harmless today
(short labels, fixed 72px column) but a latent "forces wider, doesn't
truncate" bug if a longer provider label is ever added.

### 7.4 MCP Servers, Skills, Startup — no changes needed

Already fully fluid: `PrimitiveListDetail` + `primitive-list-detail.scss`
use `width: 100%` throughout, no fixed px widths at any level; their
`<pre>` detail fields use `white-space: pre-wrap; overflow-wrap: break-word;`
correctly. Startup has no fixed-width content at all. Confirmed via full
read of all three tabs' `.tsx`/`.scss` — nothing to change.

### 7.5 Priority / sequencing

1. §7.1 (root cause) first — fixes the reported symptom on every tab at
   once, and is a small, mechanical, low-risk change (two CSS lines + one
   class attribute).
2. §7.2 (Memories) — the bigger piece; real component restructuring plus one
   small, deliberate behavior change (drop auto-select). Worth landing
   separately from §7.1 for a cleaner diff/review, but both are needed for
   the "no scrolling on any tab" bar to actually hold.
3. §7.3 (Accounts) — the row-reflow breakpoint value is a judgment call
   (§ itself proposes ~440-460px as a starting point); smallest blast radius
   of the three, touches only `_identity-panel.scss` + one new container
   query.
