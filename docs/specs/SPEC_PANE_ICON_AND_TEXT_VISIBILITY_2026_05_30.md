# SPEC: Pane Icon and Text Visibility Pass

**Date:** 2026-05-30
**Status:** Draft
**Owner:** UI / Frontend
**Scope:** pane header controls (close, magnify, mic, etc.), top-bar hamburger, pane body text size

---

## 0. Constraint (hard)

**No panel grows in height.** Header height, tab-bar height, and hamburger-button height all stay at their current values. The whole spec is an *inside-the-box* fatten — icons get larger inside the same chrome, deliberately producing a tight, dense feel rather than letting the slots breathe out. Any change below that would force a height bump is wrong and must be redesigned to fit.

---

## 1. Problem

Two readability complaints converge on the same root cause — chrome that was sized for a 96 DPI 1080p monitor circa 2018 and never grew up with the rest of the UI:

1. **Pane header icons are too small.** `--header-icon-size: 14px` glyphs (FontAwesome `xmark-large`, `MagnifyIcon`, mic, copilot, etc.) sit in a 16-px-wide slot inside a 33-px-tall header. At normal viewing distance on a high-DPI monitor they read as a single faint pixel cluster, not as a *button*. They are missed by eye-scan, fail Fitts's-law target sizing (10.5 px effective hit target after the 0.7 opacity dim), and look out of place next to the much chunkier widget bar and tab chrome.
2. **The hamburger (`.hamburger-btn`) is a 14×12 SVG inside a 28×27 button** — the icon itself is only ~50% of the available chrome, so the most important navigation entry in the app reads as a piece of UI lint rather than a primary action.
3. **The widget bar (action-widgets) is the visual reference that everything else fails to match.** Per the user, *widgets stay put* — they already feel right. The fix is to bring the pane controls and hamburger *up to* the widget bar's visual weight, not the other way around.
4. **Pane body text is undersized** — terminal/agent panes default to ~12–13 px (`var(--termfontsize, 13px)`). On a 27"+ monitor that is below the comfortable read-without-leaning threshold (~14–15 px Inter / Hack equivalent).

This spec collects the changes to (a) fatten pane-control + hamburger icons so they *comfortably fill* their slot, and (b) raise pane body text to the most-prominent-visibility default.

**Out of scope (deliberately):** the widget bar. Per user direction, widgets keep their current sizing and stay the visual anchor.

---

## 2. Research: Best Practices for "Fat, Comfortably Filling" Icons

The phrase "comfortably fill" maps to three established design tokens:

### 2.1 Target size (Fitts's law / accessibility)

| Source | Recommended touch/click target |
|---|---|
| WCAG 2.2 (2.5.8 Target Size Minimum, AA) | **24×24 CSS px** minimum |
| WCAG 2.2 (2.5.5 Target Size, AAA) | 44×44 CSS px |
| Apple HIG (macOS) | 28×28 pt for menu-bar buttons |
| Microsoft Fluent UI | 32×32 px for standard icon buttons |
| Material Design 3 | 40×40 dp icon button (24 dp icon + 8 dp padding) |

A pane header icon at 14 px in a 16 px slot is **below every published minimum**. The slot itself (24 px wide) is borderline AA — only because hover overlap helps. The fix is to fill the slot, not enlarge the header.

### 2.2 Icon-to-button optical-fill ratio

Industry consensus from Material 3, Fluent 2, and Carbon: **the glyph should occupy 55–70% of the button's content box**. Below 50% the icon "floats" and reads as undersized; above 75% it crowds and looks like a stretched bitmap.

Current pane controls:
- Header icon button slot: 24 px wide × ~28 px tall content box
- Icon: 14 px
- Fill ratio: **~50% (under-filled)** ← root cause of the "small icon" feel

Current hamburger:
- Button: 28×27 px
- SVG: 14×12 px
- Fill ratio: **~50% (under-filled)**

Target fill ratio: **62–65%** (Material's icon-button default).

### 2.3 Stroke weight ("fat")

FontAwesome 6's three styles cover the weight axis:
- `fa-light` — 1 px stroke, "dainty"
- `fa-regular` — 1.5 px stroke
- `fa-solid` — 2 px stroke, filled shapes — **what we want for header controls**
- `fa-sharp-solid` — 2 px stroke, square endcaps — **what `.connstatus` already uses**

The codebase already uses `fa-sharp fa-solid` for some elements (`block.scss:420`, `block.scss:627`). Standardizing pane controls on `fa-sharp-solid` aligns with that and delivers the "fat" look without a custom icon set.

### 2.4 Opacity / contrast

The current `.wave-iconbutton { opacity: 0.7 }` rest state drops contrast on already-small icons. With larger glyphs we can keep the dimming subtle (0.85 rest, 1.0 hover) so the icon reads as a button at rest, not just on hover.

---

## 3. Design Decisions

### 3.1 Header icon sizing (pane controls)

**Header height stays at 33 px.** Per user direction, no panel grows vertically — the whole point is that the icons get fat enough to look *tight* in the existing slot, not that the slot enlarges to accommodate dainty icons. This is a pure inside-the-box bump.

The 33 px header has ~28 px of vertical content box after the 5 px padding/border budget. An 18 px glyph in that box is ~64 % fill — well inside the comfort zone, with no clipping risk and no need to grow the chrome.

| Token | Current | Proposed | Notes |
|---|---|---|---|
| `--header-height` | 33 px | **33 px (unchanged)** | locked — no vertical growth |
| `--header-icon-size` | 14 px | **18 px** | +4 px — fills the slot |
| `--header-icon-width` | 16 px | **20 px** | header view-icon slot (kept under 22 so header text doesn't ellipsis earlier on narrow panes) |
| `.block-frame-end-icons .wave-iconbutton` width | 24 px | **26 px** | +2 px — tightens optical fill without forcing the close/magnify pair to push the title out |
| `.block-frame-end-icons .wave-iconbutton` icon font-size | inherits (14) | **18 px** | comfortable fill (~70% in the 26 px slot) |
| `.wave-iconbutton` rest opacity | 0.7 | **0.85** | icon reads as button at rest |
| `.wave-iconbutton` hover opacity | 1.0 | **1.0** | unchanged |
| End-icon icon family | `fa-solid` | **`fa-sharp fa-solid`** | "fat" stroke, aligned with existing `connstatus` icons |

The view-icon dim (`.block-frame-view-icon i { opacity: 0.5 }` in `block.scss:110`) stays at 0.5 — the view icon is a *label*, not a control, and the dim is intentional. Only interactive end icons get the new opacity.

**Vertical-fit verification:** an 18 px FontAwesome glyph's actual rendered bbox at the default line-height-normal is ~21–22 px tall. The 33 px header minus 1 px top/bottom padding (`var(--space-1)` × 2 = ~8 px combined) minus 1 px border leaves ~24 px of content height. 22 < 24 ✔. No header growth needed.

### 3.2 Hamburger sizing

Same constraint as 3.1 — **the button does not grow vertically.** The tab bar's height is fixed; the hamburger lives inside it. We fatten the bars *inside* the existing 27 px button.

| Token | Current | Proposed | Notes |
|---|---|---|---|
| `.hamburger-btn` width | 28 px | **28 px (unchanged)** | locked |
| `.hamburger-btn` height | 27 px | **27 px (unchanged)** | locked — tab bar height holds |
| Inline SVG `width`/`height` | 14×12 | **18×14** | bigger glyph inside the same button (~64% width fill, ~52% height fill — tight without crowding) |
| SVG `viewBox` | `0 0 14 12` | **`0 0 18 14`** | bars 14×3 each, 5 px stride |
| Rect height | 2 | **3** | thicker bars, "fat" |

The inline-SVG-with-rects approach (added because icon-font `fa-bars` had subpixel issues, per the comment in `tabbar.scss:115`) is kept — we just scale the rects up. Rect-fill stays uniform at any zoom.

### 3.3 Pane body text — "most prominent visibility default"

Two options for how to make pane text more visible. Pick the one whose defaults are loudest:

**Option A — bump the default size token (recommended).**
Default terminal/agent font size goes from 12/13 → **15** for first-launch users; existing per-pane and per-setting overrides win. This is the "most prominent" choice because it changes what every new pane looks like out of the box, not behind a toggle.

| Token | Current | Proposed |
|---|---|---|
| `term:fontsize` default (`termSettingsMenu.ts:14`, `termViewModel.ts:252`) | 12 | **15** |
| `--termfontsize` fallback (`agent-view.scss:42`) | 13 | **15** |
| `term.scss:200` `font-size: 10px` (status row) | 10 | **12** |

**Option B — line height + weight bump only.** Keep size, but bump weight (400 → 500) and line-height (`normal` → 1.4). Less risky for layouts that assume the current size, but also far less *prominent* — fails the user's "most prominent visibility" criterion.

→ Go with **A**. Layouts that genuinely break at 15 px were already broken at 13 px on a 4K monitor.

### 3.4 What does NOT change

- **Widget bar / action-widgets** — unchanged. `.widget-icon.text-sm` (12 px) and pinned widget sizing stay. This is the visual anchor the rest of the chrome converges toward.
- **Tab bar, tab text, tab close-button** — out of scope.
- **Modal / dropdown icons** — out of scope.
- **The `MagnifyIcon` and `MicButton` *components*** — they consume `currentColor` and `font-size`, so the parent slot bump (3.1) auto-resizes them. No component edits.

---

## 4. Files Touched

| File | Change |
|---|---|
| `frontend/app/theme.scss` | `--header-icon-size` and `--header-icon-width` only — **`--header-height` is NOT touched** |
| `frontend/app/block/block.scss` | `.block-frame-end-icons .wave-iconbutton` width + icon size; verify view-icon dim is preserved |
| `frontend/app/element/iconbutton.scss` | `.wave-iconbutton` rest opacity 0.7 → 0.85 |
| `frontend/app/block/blockframe.tsx` | Default end-icon `icon` strings: prefix `xmark-large`, `circle-plus` with the sharp-solid class (via `makeIconClass` — confirm it already routes `fa-sharp` for "solid sharp" entries, otherwise pass an explicit prefix) |
| `frontend/app/tab/tabbar.tsx` | Inline SVG `width`/`height`/`viewBox`/rect dims for hamburger |
| `frontend/app/tab/tabbar.scss` | `.hamburger-btn` width/height |
| `frontend/app/view/term/termSettingsMenu.ts` | Default font size 12 → 15 |
| `frontend/app/view/term/termViewModel.ts` | Default font size fallback 12 → 15 |
| `frontend/app/view/agent/agent-view.scss` | `--termfontsize` fallback 13 → 15 |
| `frontend/app/view/term/term.scss` | Status row 10 → 12 |

No Rust changes. No backend changes. No new components.

---

## 5. Validation

1. **Visual comparison screenshots** at 100 %, 125 %, and 150 % CSS zoom on:
   - A docked terminal pane (header + body)
   - A magnified agent pane (header end-icons especially)
   - The tab bar (hamburger should now read as a primary control, not lint)
2. **Hit-target measurement**: Chromium DevTools "show layout" → confirm every header icon button content box is ≥ 28×28 CSS px (WCAG 2.2 AA target-size minimum, with margin).
3. **Optical fill ratio**: glyph bbox / button content box for at least the close, magnify, and hamburger icons should fall in 60–68 %.
4. **Regression sweep**: run `task dev`, open a tab full of mixed pane types (term, agent, browser, sysinfo, editor), verify the header layout still ellipsises correctly when a pane is narrowed below 200 px (the new bigger end-icons must not push the view name out of view earlier than before — if so, drop `--header-icon-width` from 22 to 20).
5. **Per-pane size override still works**: open term settings → font size → 10, confirm one pane shrinks but the global default is unchanged at 15.

---

## 6. Open Questions

- Should the hamburger lift its color from `--accent-color` → `--main-text-color`? Right now it's accent-colored, which makes it stand out *despite* being small. Once it's properly sized, accent may be overkill. **Recommendation:** leave accent for now; revisit after the size bump lands and we see whether it still feels right.
- Do we want a settings toggle (`ui:large-icons`) that lets users opt *out* of the bump if their workflow depends on dense headers? **Recommendation:** no — one default, no flag. We can add it later if a real user complaint shows up.
- Do non-default themes override any of these tokens? Grep `frontend/public/themes/*.css` for `--header-icon-size` / `--header-height` before landing. If any theme hard-codes the old values, update there too.
