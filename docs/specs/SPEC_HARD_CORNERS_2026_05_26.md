# SPEC: Hard corners on buttons and modals

**Date:** 2026-05-26
**Author:** AgentA (Claude Opus 4.7)
**Scope:** Frontend visual refresh — flatten rounded corners on interactive surfaces (buttons, modal panels, dialog chrome) to a sharp, square aesthetic. Pills and avatars stay round.

---

## TL;DR

AgentMux currently uses a rounded-corner radius scale of `3 / 6 / 10 / 16 px` (`--radius-sm` → `--radius-xl`) plus `--radius-full` (9999px) for pills/avatars. The active visual direction is to **drop the corner rounding on all buttons and modal chrome** for a sharper, more terminal-native look. Pills (avatars, status chips, badges) keep their `--radius-full` treatment so they remain visually distinct from action buttons.

**The change is a token edit, not a 265-file refactor.** The radius tokens already centralize the rounding; setting the small/medium tokens to `0` propagates instantly. The minority of call sites that hard-code radii in pixels (~150 of 265 usages) are audited in a one-pass sweep and moved to either the token or `0` as appropriate.

---

## 1. Current state

`frontend/app/theme.scss` defines:

```scss
--radius-sm:   3px;   // chips, small buttons
--radius-md:   6px;   // standard buttons, inputs, cards
--radius-lg:   10px;  // modal panels, large containers
--radius-xl:   16px;  // hero cards
--radius-full: 9999px; // pills, avatars, status dots
```

Distribution (frontend):

| Form | Count | Notes |
|---|---|---|
| `var(--radius-sm)` | 5 | Token-correct, easy to flatten |
| `var(--radius-md)` | 4 | Token-correct |
| `var(--radius-lg)` | 1 | Token-correct |
| **Hard-coded `border-radius: <Npx>`** | ~255 | Need audit; most are buttons/cards/inputs |
| Total | 265 | |

Most components were written before the token scale was introduced, so they hard-code 4/6/8px values directly.

---

## 2. Visual contract

| Surface | Before | After |
|---|---|---|
| Standard buttons (primary, secondary, ghost) | 6px | **0** |
| Modal panel | 10px | **0** |
| Modal close button, modal-btn (confirm/destructive) | 4–6px | **0** |
| Input fields, search bars | 4–6px | **0** |
| Cards, popovers, dropdown menus | 4–8px | **0** |
| Block frames (pane chrome) | varies | **0** |
| Hover-highlight backgrounds | 4–6px | **0** |
| Focus rings | rounded inset | **0 (square)** |
| **Pills, avatars, status dots, badges** | 9999px | **9999px (unchanged)** |
| **Color swatches, image thumbnails** | 4–8px | **unchanged** (decorative, not interactive) |
| **Window edge** | OS-managed | **unchanged** |

Sharp corners across all *interactive* elements. Curved corners only where they convey "this is round on purpose" (pills, avatars, certain decorative elements).

---

## 3. Implementation

### Step A — token flip (low-risk, instant propagation)

In `frontend/app/theme.scss`:

```scss
--radius-sm:   0px;
--radius-md:   0px;
--radius-lg:   0px;
--radius-xl:   0px;
--radius-full: 9999px; // unchanged
```

This catches the 10 call sites already using the token scale. No downstream changes needed for those.

Optional: rename `--radius-sm/md/lg/xl` → `--radius-flat` (single token = 0) since they all collapse to the same value. Skip the rename in this pass to keep the diff minimal; do it in a follow-up if the squarification sticks.

### Step B — hard-coded-radius audit + sweep

Run:

```sh
grep -rn "border-radius:" frontend/ --include "*.scss" --include "*.css" \
  | grep -v "var(--radius-full)" \
  | grep -v "9999px" \
  | grep -v "50%"
```

For each match, classify:

1. **Interactive surface** (button, input, card, modal, popover, dropdown, hover-bg) → replace value with `0`.
2. **Pill/avatar/status** (`border-radius: 9999px` or `50%`) → leave alone.
3. **Decorative / image** (color swatch, thumbnail, sticker) → leave alone for now; designer call.
4. **Border-radius on a `<canvas>` or terminal cell** → leave alone; might be used for visual mask.

Bulk replace `border-radius: 4px;` / `6px;` / `8px;` → `border-radius: 0;` only on interactive surfaces. Use `var(--radius-md)` where the pattern is already token-aware so future flexes are easy.

### Step C — focus rings

The `--shadow-focus-ring` token (and the few `outline: ... ;` rules) often pair with rounded corners. With corners at 0, the focus ring should also be a sharp rectangle. Verify:

- `:focus-visible` outlines: stay solid, no `border-radius` needed since they follow the host's box.
- `box-shadow` focus rings (`var(--shadow-focus-ring)`): no change needed; they wrap the host's actual corner.

### Step D — manual verification surfaces

Visit and visually confirm sharp corners (no regressions) on:

- Title bar buttons (min/max/close, widget bar, More dropdown)
- Tab bar tabs (corners stay slightly rounded by design — call out exception below)
- Block frame (pane chrome)
- Hamburger menu (≡)
- Settings menu / context menus
- ConfirmModal, AgentLaunchModal, AgentInstallModal, all `<Modal>` panels
- Status bar popovers (Host, Backend, Lan)
- Search bar (Ctrl+F)
- Agent pane: identity dropdown, memory dropdown, OAuth panel, tool blocks
- Browser pane: address bar, find-on-page

### Exception — tab bar tabs

The chrome-style tabs at the top use a custom shape (`border-radius: 8px 8px 0 0;` — top-rounded only) that's part of the tab metaphor. **Tabs keep their top rounding.** Without it, tabs visually merge with the tab body. This is the one place where rounded corners convey structural meaning (tab is a "card sticking up out of the bar"), so leave alone.

---

## 4. Risks and edge cases

- **Loss of visual hierarchy:** without rounding, similar-colored buttons in adjacent rows can blend. Mitigation: rely on existing border + hover backgrounds for separation; consider increasing `--border-color` contrast if separation feels weak after the sweep.
- **Pill vs button confusion:** with most rounding gone, the `--radius-full` pills become very visually distinct. Likely fine (the whole point) but verify on the status-bar popover row where pills and buttons coexist.
- **Image thumbnails with `border-radius: 4px`:** the SPEC explicitly leaves these alone. Designer can decide later if those should also flatten.
- **macOS / Linux native chrome:** title bar buttons on macOS are traffic lights (round) and Linux are typically rounded. These are OS-managed (when traffic-light controls are enabled per PR #444). Not touched by this spec.

---

## 5. Out of scope

- Renaming the radius tokens to a single `--radius-flat` (follow-up).
- Decorative radii on images / color swatches.
- Tab shape (preserves its own pattern).
- Round avatars (preserved by design).

---

## 6. Delivery

Single small PR:

- `frontend/app/theme.scss` — token flip (sm/md/lg/xl → `0px`).
- ~25-50 grep/edit sweeps across `frontend/**/*.scss` for hard-coded radii on interactive surfaces.
- Visual smoke per §3D.
- This spec rides with the code in the same PR (`feedback_no_doc_only_prs`).

Reviewer flag: this is a visual change; eyes on the running build matter more than diff math. Squash + ship with the spec attached.
