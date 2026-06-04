# SPEC — Workspace tabs as a continuous surface with their content ("folder" model)

**Status:** Draft · **Author:** AgentA · **Date:** 2026-06-03
**Builds on:** `SPEC_TAB_BAR_FIRST_PRINCIPLES_2026_04_25`, `SPEC_WORKSPACE_TAB_SIZING_2026-05-27`, `SPEC_TAB_GAPS_AND_NAMING_2026_04_25`
**Related work in flight:** PR #1262 (responsive tab/widget collapse) and the uncommitted VSCode-look pass (flush separators, regular weight, active-bg = `--block-bg-color`, top accent, always-on close on the active tab).

---

## 1. Problem

The workspace tab strip reads as a **separate, floating object** above the panes. In VSCode the active tab and the editor below it read as **one continuous surface** — the active tab "opens" into its content the way a manila folder's tab is part of the folder. Our tabs look *disconnected*: a strip of pills hovering over a different-colored content area.

Goal: restructure the tab-bar ↔ content boundary so the **active tab + its content form a single visual surface**, while the tab strip and inactive tabs read as a recessed background. Match VSCode's default (accent on **top**, active tab merges **downward** into content).

This is an **architecture** change (surfaces + a boundary that breaks under the active tab), not a per-property CSS tweak. The incremental CSS pass already landed the easy wins (flush separators, weight, color-match); this spec covers the structural part that the tweaks can't reach.

---

## 2. Current architecture (measured)

### 2.1 Hierarchy (tab strip → content)
```
.window-header (33px, window-header.tsx:40)
  └─ TabBar (window-header.tsx:49)
       └─ .tab-bar  (tabbar.scss:6 — NO border-bottom)
            └─ .tab-bar-scroll
                 ├─ .tab-drop-wrapper › .tab[.active]      (tab.scss)
                 └─ .tab-separator                          (now flush 1px hairline)
— no gap, no border —
[div.flex-row.flex-grow]  (workspace.tsx:32)
  └─ TabContent  (tabcontent.tsx)
       └─ [div style="padding-top:1px"]  (tabcontent.tsx:96)   ← 1px shim
            └─ TileLayout  (multi-pane, flex grid WITH gaps)
                 └─ .block › .block-frame-default-inner       (block.scss:75)
```

### 2.2 Surfaces (theme.scss)
| Layer | Element | Background | Resolved |
|---|---|---|---|
| Window header / strip | `body` (inherited by `.window-header`, `.tab-bar`) | `--main-bg-color` | `rgba(34,34,34, var(--window-opacity))` — `0.45` translucent default, `1` when window:transparent=false |
| Inactive tab | `.tab` | transparent | shows the header bg through |
| **Active tab** | `.tab.active .tab-inner` | `--block-bg-color` | `rgba(0,0,0,0.5)` |
| **Pane** | `.block-frame-default-inner` | `--block-bg-color` | `rgba(0,0,0,0.5)` |
| Tile gaps | TileLayout gap | *inherits* | currently leaks the **light** header/body bg |
| Accent | `::after` / box-shadow | `--accent-color` | `rgb(65,159,224)` |
| Divider | `--tab-separator-color` → `--border-color` | `rgba(255,255,255,0.16)` |

`--secondary-bg-color` is **referenced** (editor strip) but **undefined** in theme.scss — it silently falls back to `--block-bg-color`.

### 2.3 The actual "disconnect" — three independent facts
1. **No boundary geometry.** `.tab-bar` has no `border-bottom`; the active tab has no overlap/negative-margin into the content. The only thing distinguishing strip from content is color contrast.
2. **The content is not a surface.** It's a `TileLayout` of multiple panes with gaps, fronted by a `padding-top:1px` shim. There is no single element representing "the content surface" for a tab to connect to.
3. **The gaps leak the wrong color.** Tile gaps (and the area behind the panes) currently resolve to the *light* header/body background, so the panes read as floating chips on a light field instead of tiles within a dark folder.

### 2.4 Editor reference (what it really does)
`editor-view.scss`: the strip has `border-bottom: 1px var(--border-color)` + a recessed bg; `.editor-tab--active` sets `background: var(--block-bg-color)` (= content) + `box-shadow: inset 0 -2px 0 var(--accent-color)`. **There is no negative-margin/border-break** — the comment claiming one is wrong. The editor's continuity is *color-match + accent only*; its strip border-bottom still visibly runs under the active tab. So the editor is a partial model, not a finished one — we should do the connection it only gestures at.

---

## 3. Target model — three surfaces + a broken boundary

```
        ┌──────────┬───────────┬─────────────────────────────┐
 strip  │ inactive │ ▔ACTIVE▔  │   (drag region / widgets)   │   ← Layer A: RECESSED
 (A)    │  (recessed, dim)     │                             │      = --tab-strip-bg
        └──────────┘           └──────────  boundary line ───┘
                    │  no line │                                  ← boundary BREAKS under active
        ┌───────────┴──────────┴────────────────────────────┐
        │                                                    │   ← Layer B: FOLDER SURFACE
 folder │   ┌────────────┐  ┌────────────┐                   │      = --workspace-surface
 (B)    │   │   pane 1   │  │   pane 2   │   gaps show B      │      (active tab shares this bg)
        │   └────────────┘  └────────────┘                   │
        └────────────────────────────────────────────────────┘
                                                                  Layer C: panes float on B
```

- **Layer A — recessed strip.** `.window-header` / `.tab-bar` get an explicit, slightly-darker-or-flatter strip background. Inactive tabs stay transparent (show A). This is the "back of the folder."
- **Layer B — the folder surface.** A real DOM element behind `TileLayout` painted with `--workspace-surface`. The **active tab shares this exact background**. The gaps between tiles reveal B (dark), so the content reads as one folder, not floating chips.
- **The boundary.** `.tab-bar` gets `border-bottom: 1px var(--border-color)` (the line between strip and folder). The **active tab overlaps it** (`margin-bottom: -1px` on `.tab.active`, or a 1px-extending `.tab-inner` background) so the line **breaks under the active tab** — the tab opens into B.
- **Accent on top.** `.tab.active` accent stripe at `top: 0` (VSCode default; already reverted in the in-flight pass).
- **Layer C — panes.** Sit on B; separated from B by their own subtle border/elevation (or simply by the gap revealing B). No pane should paint the light bg anywhere.

Net effect: active tab + B = one continuous dark folder with the tab as its labelled lip; A is the recessed frame; the boundary line is whole except where the active tab cuts through it.

---

## 4. Why a new surface is required (not just CSS on the tab)

Because the content is multi-pane, "the active tab's background = the content" is insufficient — there's no single content rectangle, and the inter-tile gaps + the area behind the tiles currently show the light bg. Without Layer B:
- the active tab is a dark rectangle that ends at the strip's bottom and then the eye hits light gaps → the "folder" illusion collapses one row down.
- there's nothing for the broken boundary to open *into*.

Layer B is the smallest structural addition that makes the connection real and robust to any tile arrangement.

---

## 5. Implementation plan (phased, each independently shippable + CDP-verifiable)

**Phase 0 — accent + tokens (trivial).**
- Active accent → `top: 0` (done in the in-flight pass).
- Add design tokens to `theme.scss`: `--tab-strip-bg` (Layer A) and `--workspace-surface` (Layer B). Start B = `--block-bg-color` so the active tab and panes already match; tune A as a recessed shade of the header.

**Phase 1 — introduce the folder surface (Layer B).**
- In `tabcontent.tsx`, wrap `TileLayout` in `<div class="workspace-surface">` painted `background: var(--workspace-surface)`; drop the `padding-top:1px` shim. Its top edge sits flush under `.tab-bar`.
- Ensure tile gaps + the area behind tiles reveal B, not the body bg (set the TileLayout container transparent so B shows; verify `tilelayout.scss`).
- **Verify (CDP):** the rectangle directly under the tab strip and the inter-tile gaps both resolve to `--workspace-surface`, no light leak.

**Phase 2 — break the boundary under the active tab.**
- `.tab-bar { border-bottom: 1px solid var(--border-color); }` (the strip/folder line).
- `.tab.active .tab-inner { background: var(--workspace-surface); }` and overlap the line: `.tab.active { margin-bottom: -1px; }` (or extend `.tab-inner` 1px). The active tab now visually merges with B.
- **Verify (CDP):** `getComputedStyle(activeTab .tab-inner).backgroundColor === getComputedStyle(.workspace-surface).backgroundColor`; the 1px border is occluded only under the active tab (`elementFromPoint` along the boundary returns the tab over the active column, the border elsewhere).

**Phase 3 — recess the strip (Layer A).**
- `.window-header` / `.tab-bar` → `background: var(--tab-strip-bg)`; confirm inactive tabs stay transparent so A reads through.
- Re-check the active-tab/strip contrast and the broken-line read.

**Phase 4 — polish + reconcile with existing features.**
- **Tab-colored tabs:** a colored active tab becomes a colored folder lip — its `--tab-color` overrides B on the *tab* only; decide whether the folder surface under a colored active tab tints to match (probably not; keep B neutral, let the colored lip + top accent carry identity).
- **Window transparency:** define B and A with explicit alpha so the folder stays coherent at `window-opacity` 0.45 and 1.0. The folder may be semi-transparent on a translucent window — acceptable, but A and B must keep their *relative* contrast at both opacities.
- **Zoom:** `.window-header` is `zoom`-compensated; the −1px overlap must survive `zoom: var(--zoomfactor)` (use the device-pixel-snap guidance from `RETRO_SUBPIXEL_RENDERING_RESEARCH_2026_04_26`).
- **Inactive hover / close button:** unchanged from the in-flight pass (hover shows a faint A-highlight; close X hidden on inactive, full-opacity on active).

---

## 6. Verification

- **CDP surface-continuity test** (extend the existing `/tmp/cdp-eval` harness): assert active-tab bg == workspace-surface bg == pane bg's family; assert no element under the strip/gaps resolves to the body/header bg.
- **Boundary-break test:** sample `elementFromPoint` across the strip's bottom edge — border visible except across the active tab's column.
- **Visual smoke:** window:transparent on/off, zoom 0.8/1/1.5, 1 tab vs many tabs, a colored active tab, an empty workspace.
- **No regressions:** `tabbar-dnd.test.ts`; the sizing/flush behavior from PR #1262.

---

## 7. Open questions (decide before Phase 1)

1. **B vs panes.** If B = `--block-bg-color` and panes = `--block-bg-color`, panes blend into B (VSCode-group-like). Do we want pane edges visible (give panes a subtle border / elevation) or fully flush into the folder? *Recommendation: subtle 1px pane border in `--border-color` so tiles are legible within the folder.*
2. **A's shade.** Recessed-darker or recessed-flatter (less alpha)? Needs to read as "behind" at both window opacities. *Recommendation: a flatter, slightly lighter strip than B so the dark folder advances.*
3. **Scope of the surface.** Does B also back floating/torn-off panes, or only the docked TileLayout? (Floating panes are separate windows — out of scope here; see the floating-pane architecture doc.)
4. **Single source of truth.** Fold `--workspace-surface` into the editor strip too (replace its undefined `--secondary-bg-color` reference), so editor and workspace share the folder vocabulary?

---

## 8. Non-goals
- Changing tab sizing/responsive behavior (covered by #1262).
- Reworking the TileLayout's gap/resize mechanics — only its *background reveal*.
- Floating/torn-off pane chrome.
