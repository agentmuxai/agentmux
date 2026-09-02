# Spec: Workspace Tab Sizing — Editor-Tab Parity

**Date:** 2026-05-27
**Author:** agent2
**Status:** Draft — pending decisions in §8
**Repo state:** `main` @ `0617f2b4`
**Scope:** The **workspace tab bar** (`.tab-bar` in the window header). The action-widgets bar (Agent / Swarm / …) is **out of scope** — those items are not touched.

---

## 1. Goal

Make the workspace tabs in the title-bar tab bar behave like AgentMux's editor-pane tabs and like VS Code's editor tabs:

1. **Each tab targets a "standard" width** (a preferred flex basis), not pure content width.
2. **Tabs shrink with per-tab ellipsis** when the bar gets crowded, instead of staying at content width and triggering horizontal scroll.
3. The shrink has a sensible floor; tabs that don't fit even after every other tab is at the floor still fall through to the existing horizontal-scroll path (no tabs are *hidden*).

This is the same UX as opening many files in the editor pane: each tab compresses a bit, labels get `…`, and the visible row stays the same.

---

## 2. Surfaces

### 2.1 In-scope — the workspace tab bar

```
.window-header                              ← title bar
  ├── WindowDrag .left
  ├── TabBar          ← scope of this spec
  │     .tab-bar
  │       .tab-bar-scroll  (flex container, currently overflow-x: auto)
  │         .tab-drop-wrapper × N   ← currently flex-shrink: 0
  │           .tab          ← currently width: auto; max-width: 200px
  │             .tab-inner
  │               .name     ← already has overflow:hidden + text-overflow:ellipsis
  │               .close
  │         .tab-separator × (N-1)
  │       .tab-bar-fill
  │     .hamburger-btn
  └── SystemStatus
        ├── ActionWidgets   ← OUT OF SCOPE (no changes)
        └── WindowActionButtons
```

Files:
- `frontend/app/tab/tab.tsx`
- `frontend/app/tab/tab.scss`
- `frontend/app/tab/tabbar.tsx`
- `frontend/app/tab/tabbar.scss`
- `frontend/app/tab/droppable-tab.tsx`

### 2.2 Out-of-scope — explicitly untouched

- `frontend/app/window/action-widgets.{tsx,scss}` — widget bar (Agent / Swarm / Drone / Warden + More button). The user has confirmed widget slots stay as today.
- `frontend/app/view/editor/editor-tab-strip.tsx` and its SCSS — already correct, used here only as the reference pattern.

---

## 3. Current behavior

### 3.1 Tab DOM + CSS

`tab.scss:6-22`:

```scss
.tab {
    position: absolute;        // overridden inside .tab-bar-scroll to position:relative
    width: auto;
    min-width: 0;
    max-width: 200px;
    height: 100%;
    // ...
}
.tab .name {                   // tab.scss:91-109
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1 1 auto;
    min-width: 0;
}
```

`tabbar.scss:42-49` overrides the bar context:

```scss
.tab-bar-scroll .tab {
    position: relative;
    opacity: 1;
    flex-shrink: 0;            // ← THE KEY LINE: tabs do not shrink
    left: unset;
    transform: none;
}
```

And `tabbar.scss:100-103`:

```scss
.tab-drop-wrapper {
    position: relative;
    flex-shrink: 0;            // ← AND this wrapper also doesn't shrink
    display: flex;
    align-items: flex-end;
}
```

So each tab is wrapped in a `.tab-drop-wrapper` (`flex-shrink: 0`) which contains the `.tab` (also `flex-shrink: 0` in this context). Both layers refuse to shrink.

### 3.2 Tab strip behavior

`tabbar.scss:16-39`:

```scss
.tab-bar-scroll {
    display: flex;
    flex: 0 1 auto;
    min-width: 0;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
}
```

Sizing model in practice:
- Tabs are content-width up to 200 px each. *(`max-width: 200px` on `.tab`.)*
- When the row's natural total exceeds the strip width, the strip becomes horizontally scrollable (scrollbar hidden — wheel/touch only).
- **Labels never ellipsize in normal use.** They have the CSS for it, but `flex-shrink: 0` on both wrapper layers means the flex engine never asks the tab to shrink, so the inner `.name`'s `1 1 auto` never gets squeezed.
- The `.tab-inner` has `padding: 0 var(--space-1-5)` and a `.close` button (always visible on active, hover-shown otherwise).

### 3.3 Width-animation history

`tab.tsx:205-214` documents a removed bug:

> The width-animation effect that used to live here wrote `--initial-tab-width` / `--final-tab-width` CSS vars from `props.tabWidth`, which has always been 0 (dead code). The companion `expand-width-and-fade-in` keyframe in tab.scss ran with `forwards`, pinning every new tab's width to 0 px (clamped up to `min-width: 60px`) for its entire lifetime.

The takeaway: `props.tabWidth` is dead-code plumbing still wired through `Tab` props. The current "tabs size to content via flex layout" was an explicit decision (per `RETRO_TAB_GAPS_ARCHITECTURE_ANALYSIS_2026_04_25.md`). This spec re-introduces shrink in a different shape: not via a JS-driven width, but via the flex contract on the wrappers (the path the prior attempt should have taken).

### 3.4 Editor tab strip — the reference

`frontend/app/view/editor/editor-view.scss:247-340` + `editor-tab-strip.tsx`:

```scss
.editor-tab-strip {
    overflow-x: hidden;        // clip; no scroll
}
.editor-tab {
    flex: 1 1 140px;           // preferred 140; can grow + shrink
    min-width: 80px;
    max-width: 200px;
    overflow: hidden;
}
.editor-tab-label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
```

- Preferred ~140 px width per tab; can grow up to 200 px when there's slack; shrinks to 80 px under crowding.
- Label ellipsizes inside the tab.
- Strip itself clips (`overflow-x: hidden`) — no scroll.

### 3.5 VS Code reference

`workbench.editor.tabSizing` ([PR #181729](https://github.com/microsoft/vscode/pull/181729), [issue #15048](https://github.com/microsoft/vscode/issues/15048), [issue #183532](https://github.com/microsoft/vscode/issues/183532)):

| Mode | Behavior |
|------|----------|
| `fit` (default) | Tabs at natural width; overflow → horizontal scroll. **This is what AgentMux's workspace tabs do today.** |
| `shrink` | Tabs shrink to varying widths based on label length, then become scrollable. |
| `fixed` | All tabs same width; shrink uniformly to a minimum; locked during cursor-over-bar so close-button hit-points stay predictable. |

The editor-pane tab strip in AgentMux is closest to `shrink` (variable widths, capped). This spec picks the same model for workspace tabs.

---

## 4. The gap

| Concern | Workspace tab today | Editor tab strip | This spec |
|---|---|---|---|
| Per-tab flex basis | None — content width | `flex: 1 1 140px` | `flex: 0 1 var(--ws-tab-basis)` (~160 px) |
| Tab `flex-shrink` | `0` (wrapper + tab) | implicit `1` from `flex: 1 1 ...` | `1` (allow shrink) |
| Min width | `min-width: 0` (irrelevant under `flex-shrink: 0`) | `80px` | `64px` |
| Max width | `200px` | `200px` | `220px` |
| Label ellipsis | CSS present, never trips | trips per-tab | trips per-tab |
| Overflow handling | `overflow-x: auto` (scroll) | `overflow-x: hidden` (clip) | Keep `auto` — scroll is the floor after shrink |
| Tab order / drag | Untouched | n/a | Untouched |

Observed effects today:
- Two open tabs with short names look mismatched (60 px vs 200 px). VS Code's experience is more uniform.
- Opening the 4-5th tab pushes the rightmost into the scroll region instead of compressing the row. The user must scroll to see all tabs, even when there's room to fit them at slightly narrower widths.
- Renaming a tab to a long name jumps every neighbour's position by the new width delta.

---

## 5. Proposed contract

### 5.1 CSS

`tabbar.scss` — let the wrapper shrink:

```scss
.tab-bar {
    --ws-tab-basis: 160px;     // see §8.1
    --ws-tab-min:   64px;
    --ws-tab-max:   220px;
    // existing rules unchanged
}

.tab-bar-scroll .tab {
    position: relative;
    opacity: 1;
    flex: 0 1 var(--ws-tab-basis);   // ← was flex-shrink: 0
    min-width: var(--ws-tab-min);
    max-width: var(--ws-tab-max);
    overflow: hidden;
    left: unset;
    transform: none;
    -webkit-app-region: no-drag;
}

.tab-drop-wrapper {
    position: relative;
    flex: 0 1 auto;                  // ← was flex-shrink: 0
    min-width: 0;                    // critical — let flex squeeze the wrapper
    display: flex;
    align-items: flex-end;
    // ... existing padding / transition rules unchanged
}
```

`tab.scss` — replace the hard `max-width: 200px` on the base `.tab` with the variable, and adjust min:

```scss
.tab {
    position: absolute;
    width: auto;
    min-width: 0;
    max-width: var(--ws-tab-max, 200px);   // fallback for usages outside .tab-bar
    // ...rest unchanged
}
```

`.tab .name` already has the right rules (`overflow: hidden; text-overflow: ellipsis; flex: 1 1 auto; min-width: 0`) — no change.

### 5.2 Sizing tokens

| Token | Value | Rationale |
|---|---|---|
| `--ws-tab-basis` | `160px` | Comfortably fits ~14-16 chars at the existing 11 px font (the rename-input also caps at 14 chars per `tab.tsx:192`). Editor uses 140; workspace tabs are typically named more verbosely so 160 is the right "standard." |
| `--ws-tab-min`   | `64px`  | `.tab-inner` padding (12 px both sides) + a ~3-char ellipsized name + 16 px close button. Keeps the close button reachable. Stays above WCAG 2.5.5 AAA 44 px target for the close button's effective hit area. |
| `--ws-tab-max`   | `220px` | Loose upper bound; long labels still ellipsize before they punch past max. Slightly wider than today's 200 px because the basis is now the typical state, not the cap. |

`flex: 0 1 basis` (not `1 1`) — tabs **do not grow** into free space. They sit at basis when there's room; they only shrink under pressure. This avoids the editor-tab behavior of fewer tabs sprawling across the full bar (which would look wrong in a title-bar context next to the widget bar and window action buttons).

### 5.3 Shrink cascade

When the bar narrows:

1. **All tabs at basis (160 px).** Headroom may exist between the last tab and the `.tab-bar-fill` filler (which absorbs slack via `flex: 1 1 auto`).
2. **Tabs shrink uniformly toward `min-width` (64 px).** Per-tab ellipsis kicks in as labels run out of room. *(New — does not happen today.)*
3. **All tabs at `min-width`, row still overflows.** `.tab-bar-scroll`'s existing `overflow-x: auto` becomes the floor — horizontal scroll re-engages. No tabs are hidden. This matches today's worst-case behavior; it just trips later.

The editor strip uses `overflow-x: hidden` instead. Workspace tabs deliberately keep `auto`: a hidden workspace tab is data-loss-grade bad (user can't find the tab they need), whereas a hidden editor tab is just a hop on the file picker. The scroll fallback is the right safety net here.

### 5.4 Hover stability (deliberately not adopting VS Code `fixed`)

VS Code's `fixed` mode locks tab widths while the cursor is over the bar so the close button stays in the same place across consecutive closes ([issue #15048 thread](https://github.com/microsoft/vscode/issues/15048)).

Workspace tabs in AgentMux:
- Don't have a "close many in a row" common workflow (workspaces typically hold a handful of tabs).
- Use a contextual close button that's only visible on the active tab and on hover for inactive tabs.

The benefit-to-complexity ratio is low. **Out of scope.** A future spec can re-evaluate if telemetry shows users closing many tabs in a row.

### 5.5 Color / accent / dirty state

All existing tab modifiers (`.active`, `.tab-colored`, dirty styling, hover/dragging backgrounds, the top accent stripe, the close button, the bouncing animation, the new-tab fade-in) live on the *inner* `.tab-inner` or on `.tab` directly. None of them depend on a hardcoded width.

`.tab.new-tab` fade-in keyframe is `opacity: 0 → 1` only (the broken `expand-width-and-fade-in` was removed). Width transitions are not part of the existing visual contract — adding `flex-basis` doesn't introduce a new animation.

### 5.6 Drag-to-reorder

`droppable-tab.tsx` and `tabbar-dnd.ts` compute insertion points from `getBoundingClientRect()` of tabs and reset on every drag-over. Variable widths already exist (long-name vs short-name today), so DnD has never depended on a fixed tab width. Tabs at basis = more *uniform* widths = the drop math is, if anything, more predictable.

### 5.7 Sub-pixel rendering

`tabbar.scss:32-34` documents a `will-change: transform; transform: translateZ(0); backface-visibility: hidden` block for sub-pixel rendering through the ancestor `zoom: var(--zoomfactor)`. Per-tab flex sizing produces fractional pixel widths during browser layout. The existing layer-promotion already covers this case (it was added specifically for `--zoomfactor` interactions which produce fractional values). **No additional treatment required.**

If sub-pixel-driven gap shimmer appears at certain zoom levels, the fix is `width: round(--ws-tab-basis, 1px)` on `.tab` (CSS rounding, supported in Chromium 123+). Mark as a follow-up if observed.

### 5.8 Tab rename + 14-char cap

`tab.tsx:192` blocks input past 14 characters during inline rename. The new basis (160 px) was sized to comfortably show a 14-char label without ellipsis at full width — so renaming-then-shrinking is the only way a label appears truncated. That's correct behavior.

### 5.9 What stays the same

- Tab DOM hierarchy and components.
- Tab separators (`.tab-separator`) and their pixel-snap math.
- Drag-to-reorder, tear-off, drop indicators.
- `.tab-bar-fill` greedy filler (`flex: 1 1 auto`) that absorbs slack on the right of the last tab.
- Hamburger button (`.hamburger-btn`) — fixed `28px` width, untouched.
- Horizontal scroll-on-overflow as the absolute floor (`.tab-bar-scroll { overflow-x: auto }`).
- `.tab-context-panel` color picker.
- Active-tab top accent stripe + background highlight.
- Tab colors (`tab:color` meta) — applied on `.tab-inner` background, width-independent.

---

## 6. Files touched (estimate)

| File | Change |
|---|---|
| `frontend/app/tab/tabbar.scss` | Introduce `--ws-tab-*` tokens on `.tab-bar`; replace `flex-shrink: 0` on `.tab-bar-scroll .tab` with the new `flex` contract; replace `flex-shrink: 0` on `.tab-drop-wrapper` with `flex: 0 1 auto; min-width: 0`. |
| `frontend/app/tab/tab.scss` | Swap the literal `max-width: 200px` on `.tab` for `max-width: var(--ws-tab-max, 200px)`. |
| `specs/SPEC_WORKSPACE_TAB_SIZING_2026-05-27.md` | This spec. *(File currently named `SPEC_WIDGET_BAR_TAB_SIZING_…`; rename on implementation since "widget-bar" was the original misread of the user's intent.)* |
| `.changesets/<ts>-feat-tabs-…md` | One-line patch changeset. |

No TS changes. No Rust changes. No prop changes (the dead-code `tabWidth` prop is intentionally left alone — out of scope, but a candidate for a separate cleanup PR).

---

## 7. Verification plan

1. `task dev`, single workspace tab — tab sits at 160 px (basis). Full label visible.
2. Open 4-6 tabs with mixed-length names — every tab sits at 160 px, no ellipsis, no scroll.
3. Keep opening tabs (or narrow the window) until the row would overflow — tabs compress uniformly toward 64 px; labels ellipsize per-tab.
4. Continue until even at 64 px the row would overflow — horizontal scroll re-engages, no tabs are hidden. (Verify mouse-wheel scrolls the bar.)
5. Active tab has the accent stripe + highlighted background regardless of width.
6. A tab with a user-set `tab:color` paints correctly at any width.
7. Hover any tab → close button (×) appears in a predictable spot inside the tab.
8. Rename a tab → width unchanged (still at basis or shrunk floor); label content updates and ellipsizes if necessary.
9. Drag a tab to reorder — drop indicator lands where expected; no visual jump.
10. Tear-off (drag past `TEAR_PAST_PX = 5`) still triggers the new-window handshake.
11. Zoom in / out — sub-pixel rendering doesn't introduce shimmer at the new flex widths (regression check against `RETRO_SUBPIXEL_RENDERING_RESEARCH`).
12. Widget bar (`ActionWidgets`) is **visually identical** to today — verify by screenshot diff.

---

## 8. Open decisions

### 8.1 Basis value (140 / 160 / 180)

- **140 px** = matches the editor tab strip exactly. Pro: consistent across both tab surfaces. Con: workspace tab labels tend to be longer than file basenames; more tabs will ellipsize even with headroom.
- **160 px** (recommended) = empirically fits 14-char names (the rename cap) without ellipsis. Workspace-specific tuning.
- **180 px** = roomy; tabs feel less compressed. Con: only 4-5 tabs fit at full basis in a 1080p title bar after the widget bar.

**Recommendation:** start at **160 px**, iterate from screenshots.

### 8.2 New user setting `tab:sizing`?

VS Code exposes `workbench.editor.tabSizing`. We could mirror with `tab:sizing` accepting `fit | shrink` (default `shrink`, fall back to `fit` = today's behavior).

**Recommendation:** **defer.** Land the shrink behavior as the new default; expose a setting only if a user reports they prefer the old scroll-only behavior. Adding a setting up front adds testing surface for an unlikely use case.

### 8.3 Floor on tab count before shrink kicks in

We could keep tabs at basis when there are ≤ 3-4 tabs (the "I have headroom anyway" case), and only enable shrink past that. This is automatic with the chosen contract — `flex: 0 1 basis` with `.tab-bar-fill { flex: 1 1 auto }` absorbing slack means shrink doesn't engage until the row would overflow. No JS gate needed.

### 8.4 Should `overflow-x: auto` become `hidden`?

Editor tabs clip. Workspace tabs should not — a hidden workspace tab is unrecoverable from the bar itself (no overflow chip in the title bar today). Keep scroll. A future "overflow chevron" affordance would change this calculus.

### 8.5 Removing dead `tabWidth` prop

`Tab` accepts a `tabWidth: number` prop (`tab.tsx:111`) that is dead code (always 0). Not relevant to this spec but worth a follow-up cleanup PR.

---

## 9. Implementation outline

Single PR, one commit:

**Commit — `feat(tabs): workspace tabs adopt editor-style basis + shrink + ellipsis`**

- Add `--ws-tab-basis: 160px; --ws-tab-min: 64px; --ws-tab-max: 220px;` to `.tab-bar` in `tabbar.scss`.
- Replace `.tab-bar-scroll .tab { flex-shrink: 0 }` with `flex: 0 1 var(--ws-tab-basis); min-width: var(--ws-tab-min); max-width: var(--ws-tab-max); overflow: hidden`.
- Replace `.tab-drop-wrapper { flex-shrink: 0 }` with `flex: 0 1 auto; min-width: 0` (preserve the existing padding/transition rules).
- Swap the hard `max-width: 200px` on `.tab` (in `tab.scss`) for `max-width: var(--ws-tab-max, 200px)`.
- No JSX / TS changes.
- Add changeset: `feat(tabs): workspace tabs adopt editor-style basis + shrink + ellipsis (160/64/220 px)`.

---

## 10. References

- `frontend/app/tab/tab.tsx` — workspace tab component
- `frontend/app/tab/tab.scss` — workspace tab base CSS
- `frontend/app/tab/tabbar.tsx` — tab strip container
- `frontend/app/tab/tabbar.scss` — tab strip CSS (the `flex-shrink: 0` site we're replacing)
- `frontend/app/tab/droppable-tab.tsx` — DnD wrapper (no change)
- `frontend/app/view/editor/editor-tab-strip.tsx` — reference editor tab strip
- `frontend/app/view/editor/editor-view.scss:247-340` — reference CSS contract
- `docs/specs/SPEC_TAB_BAR_FIRST_PRINCIPLES_2026_04_25.md` — separator architecture + sizing history
- `docs/retro/RETRO_TAB_GAPS_ARCHITECTURE_ANALYSIS_2026_04_25.md` — why the old width-animation was removed (avoid repeating that mistake)
- `docs/retro/RETRO_SUBPIXEL_RENDERING_RESEARCH_2026_04_26.md` — sub-pixel + `--zoomfactor` interactions
- VS Code [`workbench.editor.tabSizing` PR #181729](https://github.com/microsoft/vscode/pull/181729)
- VS Code [issue #15048 — allow shrink-instead-of-min-width](https://github.com/microsoft/vscode/issues/15048)
- VS Code [issue #183532 — test `fixed` sizing](https://github.com/microsoft/vscode/issues/183532)
- VS Code [Custom Layout docs](https://code.visualstudio.com/docs/configure/custom-layout)
- WCAG 2.5.5 — [Target Size (AAA)](https://www.w3.org/WAI/WCAG21/Understanding/target-size.html)
