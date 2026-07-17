# Second-level submenus escape the paintable-area framework

**Date:** 2026-07-16
**Symptom (user report):** right-click a pane header → hover **"Replace With..."** — the
second-level submenu gets cut off at the window edge. The top-level menu is always
placed correctly.
**Verdict:** the paintable-area framework is working as designed; the pane
context-menu renderer's **second level never calls it**. The submenu is positioned by
two lines of hand-rolled CSS from before the framework existed.

---

## 1. What the framework guarantees (and to whom)

`frontend/app/util/menu-position.ts` (SPEC_MENU_PAINTABLE_AREA_GUARD_2026_05_20) is
opt-in per call site. A surface that calls `computeMenuPosition(request, el)` gets the
floating-ui pipeline — `offset → flip → shift → size` against an explicit boundary
(paintable area or viewport) — and can call `assertMenuInPaintableArea(el, label)` to
get the dev-only `[menu-guard]` console error if it still lands outside. A surface
that doesn't call it gets nothing: there is no global observer sweeping the DOM for
off-screen menus.

Surface inventory as of today:

| Surface | Level 1 | Level 2 | Dev guard |
|---|---|---|---|
| `FlyoutMenu` (`element/flyoutmenu.tsx`) | ✅ `computeMenuPosition` (:77–81) | ✅ `right-start`/`left-start` compute (:395–403) | ✅ (:92, :414) |
| `Popover`, `Tooltip` | ✅ (Phase 2/3 migrations) | n/a | ✅ |
| `components/context-menu.tsx` | ✅ | n/a — "Single-level only" by contract | ✅ |
| **`showJsContextMenu`** (`util/cef-api.ts` — ALL pane right-click menus) | ✅ (:244–260) | ❌ **hand-rolled CSS, no framework call** | ❌ never called for submenus |

`showJsContextMenu` is the renderer behind `showContextMenu` (`cef-api.ts:306–309`),
which is what `store/contextmenu.ts` drives. Every pane right-click menu goes through
it — so every submenu built via `type: "submenu"` is affected: **"Replace With..."**
(`block/pane-actions.ts:101–136`), **"Pane Color"** (`block/blockframe.tsx`), and any
future menu def with a `submenu` array.

## 2. Root cause — `cef-api.ts:199–220`

When `renderItems` meets an item with a submenu it does this:

```ts
row.style.position = "relative";          // :205
const sub = document.createElement("div");
sub.className = "menu sub-menu";
sub.style.display = "none";
sub.style.left = "100%";                  // :215 — anchor at row's right edge
sub.style.top = "0";                      // :216 — align with row's top
renderItems(sub, item.submenu);
row.appendChild(sub);
row.addEventListener("mouseenter", () => { sub.style.display = ""; });
row.addEventListener("mouseleave", () => { sub.style.display = "none"; });
```

`position: absolute; left: 100%; top: 0` inside the hovered row
(`.menu` is `position: absolute` from `flyoutmenu.scss:9`), unconditionally. There is:

- **no `computeMenuPosition` call** — no flip to `left-start` when the right side has
  no room, no shift, no size clamp;
- **no `assertMenuInPaintableArea` call** — so the framework's own dev alarm is
  structurally blind here. This is why the regression survived the Phase 4 guard:
  the guard only covers surfaces that opt in;
- **no vertical handling** — a tall submenu near the bottom edge overflows downward
  the same way (`top: 0` pins it to the row).

Because the parent menu is placed at the cursor and is up to 400px wide
(`flyoutmenu.scss:12`), right-clicking anywhere in the right ~half of the window puts
the row's right edge close enough to the window edge that the submenu (width:
`max-content`, `flyoutmenu.scss:20–23`) extends past it. DOM content cannot paint
outside the window, so it's clipped — exactly the reported "Replace With... gets cut
off".

Contrast with the compliant implementation in the same codebase:
`FlyoutMenu`'s `SubMenu` (`flyoutmenu.tsx:395–414`) computes
`computeMenuPosition({ anchor: anchorRect, placement: mirrored ? "left-start" :
"right-start", avoidNativePanes: false })` with `autoUpdate`, then asserts the guard.
That is the pattern the context-menu submenu was supposed to follow and didn't —
`showJsContextMenu`'s **top level** was migrated to the framework (with careful
visibility:hidden-until-placed handling, `cef-api.ts:244–260`), but the submenu branch
kept its pre-framework CSS.

## 3. Secondary gap (both renderers): `maxWidth`/`maxHeight` are computed and dropped

`computeMenuPosition` returns `{ style, placement, maxHeight, maxWidth }` — the `size`
middleware's clamp for when even a flipped placement can't fully fit. Neither consumer
applies it:

- `flyoutmenu.tsx:22` `styleToString` serializes only `position/left/top`;
- `cef-api.ts:253–254` copies only `pos.style.left` / `pos.style.top`.

So even framework-routed menus can overflow when they're taller than the paintable
area (they flip and shift, but never shrink/scroll). Low severity — it needs an
unusually tall menu — but it's the same class of bug and the fix should thread it
through (`max-height` + `overflow-y: auto`). `maxWidth` is deliberately left
unapplied: an inline max-width overrides (and can loosen) the `.menu` 400px CSS cap,
and flip+shift already guarantee horizontal fit for menus at or under that cap.

## 4. Fix direction

In `showJsContextMenu`'s submenu branch, replace the static CSS with the FlyoutMenu
pattern, adapted to the imperative DOM style of this function:

1. Keep the DOM nesting (it's what makes the `mouseenter`/`mouseleave` hover logic
   work), but position the submenu with **`position: fixed`** viewport coordinates
   instead of `left:100%` — `computeMenuPosition` already returns
   `position: fixed` styles. (Caveat: fixed coords resolve against the viewport only
   if no ancestor creates a containing block via `transform`/`filter`; the overlay
   chain here — plain `.menu` inside a `position:fixed` overlay — is clean today.)
2. On `mouseenter`: show it `visibility: hidden`, call
   `computeMenuPosition({ anchor: row.getBoundingClientRect(), placement:
   "right-start", avoidNativePanes: false }, sub)`, apply `left/top` **and**
   `maxHeight` (with `overflow-y: auto`; see §3 for why not `maxWidth`), then
   reveal — same
   hidden-until-placed discipline the top level already uses (it also keeps the
   `data-pane-overlay` clip rect from registering a stale position).
3. Call `assertMenuInPaintableArea(sub, "context-submenu")` after placement so the
   dev guard covers this surface from now on.
4. flip handles the left-mirroring automatically (`right-start` → `left-start`), same
   as FlyoutMenu's `mirrored` case.

Optionally, apply the returned `maxHeight`/`maxWidth` in `flyoutmenu.tsx`'s
`styleToString` too (§3) — same PR, few lines.

**Repro to verify against:** move a window so its right edge is near the screen's
right, right-click a pane header in the window's right half, hover "Replace With..."
(long list — every pane widget) and "Pane Color". Both must flip to the left of the
parent menu and stay fully visible; in dev, no `[menu-guard]` console errors.

## 5. Sources

- `frontend/util/cef-api.ts` :199–220 (broken submenu branch), :244–260 (compliant top level)
- `frontend/app/element/flyoutmenu.tsx` :77–92 (L1), :395–414 (compliant SubMenu pattern)
- `frontend/app/util/menu-position.ts` (framework), `flyoutmenu.scss` :7–23 (`.menu`, `.sub-menu`)
- `frontend/app/block/pane-actions.ts` :101–136 ("Replace With..." def), `store/contextmenu.ts` :45–46 (submenu conversion)
- `docs/specs/SPEC_MENU_PAINTABLE_AREA_GUARD_2026_05_20.md` (framework spec; Phase 3 "manual-math surfaces" is where this site was missed)
