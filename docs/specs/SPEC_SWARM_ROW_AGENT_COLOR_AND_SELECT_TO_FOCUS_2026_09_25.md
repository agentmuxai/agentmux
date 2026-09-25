# SPEC: Swarm rows use the agent's own pane-tab color, and selecting a row focuses its pane instead of the Swarm pane

**Date:** 2026-09-25
**Status:** proposed
**Trigger:** Repo owner: *"when selected in the swarm, the border color in the
swarm when selected should be the same as the agent's pane tab's selected
color. when hovering over the swarm, the hue of each entry should match the
lighter version of that agent's color. also, we want to change the select
behavior. currently if a usr selects an agent on the swarm list, it selects
the entire pane. lets change that. if the user clicks in the header or an
area not an agent entry, it selects the pane. but if you select an agent,
the agent border for the swarm entry is highlighted, and that also brings
the pane into focus. you also need to tweak the pane with multiple tabs.
(the hover open button should be removed, simply selecting the agent does
that operation)"*
**Scope:** `frontend/app/view/swarm/swarm-view.tsx`,
`frontend/app/view/swarm/swarm-view.scss`, `frontend/app/view/agent/agent-color.ts`.
No backend changes.

---

## 1. Current behavior, exactly as it is today

| Piece | Where | What it does now |
|---|---|---|
| Row click | `swarm-view.tsx:296` — `onClick={() => node.blockId && model.toggleAgentCollapsed(node.blockId)}` | Toggles the collapsed/expanded state of the agent's subtree. Does **not** focus anything. |
| Hover "open" button | `swarm-view.tsx:336-345`, class `.swarm-agent-focus` | `e.stopPropagation()` then `void focusBlock(node.blockId)` — the only place in this file that currently focuses the target agent's own pane. |
| Selected-row border | `swarm-view.scss:112-119`, `.swarm-agent-card--active` | `box-shadow: inset 0 0 0 1px var(--accent-color, #5b8dd9)` — one fixed theme accent color for every agent, regardless of that agent's own color. |
| Hover background | `swarm-view.scss:108-110` | `background: var(--highlight-bg)` — a generic theme tint, same for every agent. |
| Why clicking a row "selects the entire pane" today | `frontend/app/block/block.tsx:218-227`, `handleBlockClick` (bound as the Swarm block's own `onClick`) | Unconditionally calls `giveBlockFocus(nodeModel.blockId)` — `nodeModel.blockId` here is the **Swarm pane's own** block id — whenever the click isn't already `stopPropagation`'d before it bubbles this far. Today's row click (`toggleAgentCollapsed`) never stops propagation, so every row click also re-focuses the Swarm pane itself as a side effect. This is exactly the behavior the trigger quote describes as the bug. |

This is a **regression, not a new ask**: `docs/specs/swarm-active-pane-sync.md`
(2026-06-23, status "Implemented — #1721") already specified *"Swarm → Pane
(already done): clicking a Swarm row focuses the corresponding pane,
switching tabs if needed."* That binding was later repurposed for
collapse-toggle; this spec restores it, on a different element, alongside
the new color work.

---

## 2. Design

### 2.1 Selected-row border = the agent's own pane-tab color

The pane tab's "selected color" already has one canonical source:
`computeBlockActiveBorderColor(blockMeta)` (`frontend/app/block/blockframe.tsx:985-994`) —
the exact function `PaneChrome.tsx`'s `tabColors` memo (~line 159) calls per
tab to paint its underline. It is hue-aware: a user's explicit `frame:hue`
override wins; otherwise it falls back to `frame:activebordercolor`, the
full-strength color `pickAgentColor(agentId)` (`agent-color.ts:43-53`)
assigned at spawn and persisted into block meta
(`SPEC_AGENT_COLOR_2026_08_08.md`). Reusing it — not
`pickAgentColor` directly — means a custom "Pane Color" override is honored
in Swarm too, not just the default agent color.

In `AgentRow`:

```ts
const blockMeta = createMemo(() => node.blockId
    ? MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", node.blockId))()?.meta
    : undefined);
const activeBorderColor = createMemo(() => computeBlockActiveBorderColor(blockMeta()));
```

Expose it as a CSS custom property on the row (`style={{ "--swarm-agent-active-border": activeBorderColor() }}`)
and change `swarm-view.scss`:

```scss
&--active {
    background: var(--highlight-bg);
    box-shadow: inset 0 0 0 1px var(--swarm-agent-active-border, var(--accent-color, #5b8dd9));
    border-radius: 0;
}
```

`computeBlockActiveBorderColor` returns `undefined` for a block with no
color set at all — the `var(--x, <fallback>)` keeps today's accent-blue for
that edge case, so nothing regresses for a row Swarm can't resolve a color
for.

### 2.2 Hover tint = a lighter version of the same color

No `lighten()`/tint helper exists in the codebase today. The only sibling
is `dimAgentColor` (`agent-color.ts:64-72`), which **darkens** each channel
by `× 0.55` for the unfocused pane border — same file, same shape, same
`isValidAgentColor` contract. Add its lightening counterpart there, as the
one home for agent-color derivations:

```ts
/** Lightened variant for the Swarm row hover tint — mixes each channel
 *  toward white by `factor` instead of dimAgentColor's darken-toward-black. */
export function lightenAgentColor(hex: string, factor = 0.45): string {
    if (!isValidAgentColor(hex)) return hex;
    const channel = (s: string) => parseInt(s, 16);
    const mix = (c: number) => Math.round(c + (255 - c) * factor);
    const r = mix(channel(hex.slice(1, 3)));
    const g = mix(channel(hex.slice(3, 5)));
    const b = mix(channel(hex.slice(5, 7)));
    const hex2 = (n: number) => n.toString(16).padStart(2, "0");
    return `#${hex2(r)}${hex2(g)}${hex2(b)}`;
}
```

In `AgentRow`, derive it from the *same* `activeBorderColor` (so a custom
hue override is honored here too, not just the default palette color):

```ts
const hoverTint = createMemo(() => {
    const base = activeBorderColor();
    return base ? lightenAgentColor(base) : undefined;
});
```

Expose as `--swarm-agent-hover-bg`, and in `swarm-view.scss`:

```scss
&:hover {
    background: var(--swarm-agent-hover-bg, var(--highlight-bg));
}
```

`0.45` is a starting point, not a hard requirement — see §7.

### 2.3 Selection behavior: a row focuses its own pane, not the Swarm pane

**Root cause, precisely** (from §1's last row): `block.tsx`'s
`handleBlockClick` is the Swarm pane's *own* generic "clicked anywhere
inside me → focus me" handler. It fires on every click that reaches it
un-stopped. Two existing controls in the same row already know to guard
against this — the fleet-select checkbox (`swarm-view.tsx:310`,
`onClick={(e) => e.stopPropagation()}`) and the hover Focus button
(`swarm-view.tsx:340`, same pattern). The row body itself is the one place
that doesn't, today.

**Fix — three small changes, one file:**

1. **Row body's `onClick`** (`swarm-view.tsx:296`) becomes:
   ```tsx
   onClick={(e) => {
       e.stopPropagation();
       if (node.blockId) void focusBlock(node.blockId);
   }}
   ```
   This is the *exact* call the hover button already makes
   (`focusBlock`, `frontend/app/util/focus-block.ts:23-64`) — no new
   focus/tab-switch logic needed. `focusBlock` already: finds the leaf
   hosting the block via `layoutModel.getNodeByBlockId` (which searches
   both a leaf's active `blockId` and its `blockStack` of background
   tabs — `frontend/layout/lib/layoutNodeModels.ts:222-231`), switches
   that leaf's active tab if the agent is a background tab in a
   multi-tab pane (`setActiveTab`, `frontend/app/store/tab-actions.ts:94`),
   and focuses the leaf (`layoutModel.focusNode`). **This is the "tweak
   the pane with multiple tabs" the trigger asks for — it is already
   built and already exercised today by the hover button; the only
   change is which element calls it.**
   `e.stopPropagation()` here is what makes the Swarm pane's own
   `handleBlockClick` never run for this click, so the target agent's
   pane stays focused instead of being immediately re-overridden by a
   Swarm-pane self-focus.

2. **Collapse/expand moves to the chevron.** Today the chevron
   (`swarm-view.tsx:319-324`) is decorative — its own comment says *"No
   own click handler — the enclosing card's click already toggles."*
   Since the card's click is being repurposed (step 1), the chevron
   needs to pick up that job itself:
   ```tsx
   <i
       class={`fa-solid fa-${collapsed() ? "chevron-right" : "chevron-down"} swarm-agent-expand-icon`}
       onClick={(e) => {
           e.stopPropagation();
           if (node.blockId) model.toggleAgentCollapsed(node.blockId);
       }}
   />
   ```
   (Only rendered `when={hasChildren()}` already — nothing to collapse
   when there's no chevron, same as today.)

3. **Remove `.swarm-agent-focus` entirely** — the button
   (`swarm-view.tsx:336-345`), its hover-reveal CSS
   (`swarm-view.scss:188-214`), and the `.swarm-agent-card:hover
   .swarm-agent-focus` / `.swarm-agent-focus:hover` rules. Its one job
   (`focusBlock`) is now the row's default click; a redundant explicit
   button is exactly what the trigger asks to remove: *"the hover open
   button should be removed, simply selecting the agent does that
   operation."*

**Header / empty-area clicks — explicitly a non-change.** `handleBlockClick`
already does the right thing for anything that isn't one of the
`stopPropagation`-guarded row controls above: it focuses the Swarm pane
itself, which is exactly *"if the user clicks in the header or an area not
an agent entry, it selects the pane."* Nothing in this spec touches
`block.tsx` — this requirement is satisfied by leaving it alone while
fixing the row body (which today incorrectly reaches the same handler).

---

## 3. What doesn't change

- The fleet-select checkbox (`SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md`) —
  already stops propagation for its own, unrelated reason (bulk actions).
- `handleBlockClick`/`giveBlockFocus` (`block.tsx`) — untouched; still what
  fires for header/empty-area clicks, per §2.3.
- `focusBlock` itself (`focus-block.ts`) — untouched; only its call site
  moves from a small hover button to the row body.
- The `--active`/`focusedBlockId() === node.blockId` binding
  (`swarm-active-pane-sync.md`, already reactive to
  `layoutModel.focusedNode()`) — reused as-is. Clicking a row now drives a
  real focus change instead of leaving that binding stale, but the binding
  itself needs no changes.

---

## 4. Non-goals

- Not changing the Swarm tree's grouping/collapse **model** (todo/tool/
  workflow buckets, long-running promotion, etc.) — only which element
  triggers collapse.
- Not introducing a new "selected but not focused" state — reusing
  `focusedBlockId` as the one source of truth for the highlighted row,
  consistent with `swarm-active-pane-sync.md`'s original design.
- Not changing `pickAgentColor`/`AGENT_COLOR_PALETTE`/color *assignment* —
  this spec only reads the already-resolved, already-stored block-meta
  color, the same way `PaneChrome` does.
- Not extending this treatment to the row's *unfocused/dimmed* state
  (today's plain row background) — see §7.

---

## 5. Phases

| Phase | Scope |
|---|---|
| **P1** | `agent-color.ts`: add `lightenAgentColor` |
| **P2** | `swarm-view.tsx`: read block meta → `computeBlockActiveBorderColor` per row; expose `--swarm-agent-active-border` / `--swarm-agent-hover-bg`; `swarm-view.scss`: swap the hardcoded `--accent-color`/`--highlight-bg` in `.swarm-agent-card--active`/`:hover` for the new vars (fallbacks preserved) |
| **P3** | `swarm-view.tsx`: row `onClick` → `stopPropagation` + `focusBlock`; chevron gets its own guarded `onClick` for collapse; remove `.swarm-agent-focus` (button + CSS) |

Small, contained diff across three files — one PR is plausible; phases are
for review clarity, not separate merges.

---

## 6. Verification

- **Unit**: `lightenAgentColor` test in `frontend/app/view/agent/agent-color.test.ts`,
  mirroring the existing `describe("dimAgentColor", ...)` block (lines
  43-58) — a known hex in, an expected lighter hex out, invalid-hex
  passthrough unchanged.
- **Manual — color**: two agents with different palette colors (and one
  with a custom `frame:hue` override, via the Pane Color picker) show
  Swarm selected-borders that exactly match their own pane tab's
  underline color; hovering shows a visibly lighter tint of that same
  hue, not the old generic `--highlight-bg`.
- **Manual — multi-tab**: an agent that is a background tab in a
  multi-tab pane, clicked from Swarm, switches that pane to its tab and
  focuses it — same outcome as today's hover button, now via the row
  (regression check on `focusBlock`'s existing behavior, not new
  behavior).
- **Manual — collapse vs. select**: clicking the chevron toggles
  collapse without moving focus; clicking the row body (not the
  chevron, not the checkbox) focuses the target pane without toggling
  collapse.
- **Manual — non-regression**: clicking Swarm's own header/empty area
  still focuses the Swarm pane itself, unchanged (§2.3, §3).

---

## 7. Open questions

- **Lighten factor.** `0.45` toward white is a starting point matching
  `dimAgentColor`'s `0.55`-toward-black shape, not a measured/signed-off
  value — needs a visual pass. A `color-mix(in srgb, <c> <n>%, white)`
  SCSS approach (precedent: `agent-view.scss:261,265`) is the
  alternative if a JS-computed hex proves harder to tune live than a CSS
  value; recommending the JS twin of `dimAgentColor` here only because it
  mirrors an existing, already-accepted pattern exactly.
- **Unfocused row color.** This spec only touches the *selected*
  (`--active`) and *hover* states, per the trigger's literal ask. Whether
  the plain, non-hovered, non-selected row background should also gain a
  faint per-agent tint (mirroring `frame:bordercolor`/`dimAgentColor` the
  way the pane border itself does) is a natural follow-up, not assumed
  here.
