# Report: the tool-call hover panel — line height, flicker when it is too tall, and leaving the pane

**Status:** analysis

Date: 2026-10-02 · Author: Loap · Base: `main` @ `f5dfeafbc` (v0.59.5)

Analysis only. No source files were changed. The flicker is reproduced by a
simulation of the placement code (§3.3), not by watching the app.

---

## 0. Summary

The panel you see when hovering a tool call is `PeekOverlay`
(`frontend/app/view/agent/components/PeekOverlay.tsx`), fed by `ToolBlock.tsx`
(tool name + the full command, plus a time/token row). Three asks:

| Ask | Finding | Size of the fix |
|---|---|---|
| 1. Narrower line height | Body text is `line-height: 1.6`, the tool-preview log beside it is `1.4`. Dropping to the shared `--leading-normal` (1.45) or the log's 1.4 is a one-line change and is worth doing. | One SCSS value. Gains ~2 lines, **not** a fix for flicker. |
| 2. Flicker when too big | **Root cause found, and it is not the panel's size by itself.** When the panel fits neither below nor above the cursor, the code clamps it to the top of the transcript, and the panel lands *on top of the cursor*. The cursor is then "outside the row", the panel closes, the cursor is back on the row, the panel reopens — a ~50 ms loop. | A placement change. Scroll bars alone do not fix it (§4). |
| 3. Extend outside the pane | Technically easy: it is already portalled to `document.body`. Three things hold it inside the pane: `max-width` = row width, height capped to the transcript's scroll container, and a "right edge = row right edge" anchor. | Replace the bounds. One new requirement (browser-pane airspace, §5). |

**Recommendation:** fix 2 and 3 together, because the cleanest cure for the
flicker *is* letting the panel leave the pane — put it **beside** the row, not
over the cursor, and the cursor can never end up inside it. Details in §6.

---

## 1. What the panel is today

- `ToolBlock.tsx:527` mounts `<PeekOverlay show={isPeeking() && hasAnyPeekContent()}>`
  with a `PeekMetaRow` (time · `~n tok (est.)`) and a body of tool name + `cmdText()`
  (`header().detail`).
- `useNodePeek.ts` is the hover state: `mouseenter` on the **row** starts a 50 ms
  timer, `mouseleave` on the row closes it immediately. It ignores enters while the
  primary mouse button is down (selection drags).
- `PeekOverlay.tsx` renders into a `<Portal>` at `document.body`, `position: fixed`,
  `autoUpdate` from floating-ui, hand-rolled placement in `update()`, and a
  pane-zoom compensation (`withPaneZoom`).
- 13 call sites share it: `ToolBlock`, `PersistentShellBlock`, `MarkdownBlock`,
  `UserMessageBlock` (×2, one `align="stretch"`), `CollapsibleMessage`,
  `AmbientNarrationBlock`, and 6 in `virtualization/DocumentRow.tsx`. Most are two
  short lines. **`ToolBlock` is the one that can be arbitrarily tall**, because the
  command is shown in full (`toolDetailOf` does not truncate; I did not audit each
  per-tool `detail` fact).

Styling is in `styles/_document-nodes.scss` (`.agent-node-peek-overlay`,
`.agent-node-peek-tooltip-body`, `.agent-node-peek-tooltip-meta`).

## 2. Ask 1 — line height

Current values:

| Rule | font-size | line-height |
|---|---|---|
| `.agent-node-peek-tooltip-body` (the command) | 13px | **1.6** |
| `.agent-node-peek-tooltip-meta` / `-note` | 13px | 1.4 |
| `.agent-tool-log-line` (the in-flow tool preview) | `--text-sm` (12px) | 1.4 |
| `--leading-normal` / `--leading-tight` tokens | | 1.45 / 1.2 |

The command body is the loosest text in the tool UI. Suggested: **1.4** (matches the
preview log it is a peek of) or the `--leading-normal` token (1.45). Also reducible:
the overlay `gap: 4px` and `padding: 2px 4px` — small, but the panel only has two
blocks.

What it buys, on an 800px-tall transcript with the cursor at mid-height (the worst
case for the current placement, §3.3): the tallest panel that does not cover the
cursor goes from **17 lines at 1.6 to 19 at 1.4**. That is useful polish and it makes
the common long-ish command fit, but it only moves the threshold. A 40-line command
still flickers. **Do it, but do not mistake it for the fix.**

## 3. Ask 2 — the flicker

### 3.1 Mechanism

`PeekOverlay` is portalled, so it is **not a DOM descendant of the row**. The row's
`mouseleave` therefore fires the moment the pointer moves onto the panel, and
`useNodePeek.handlePeekLeave` unmounts it. The code knows this — it keeps the panel
`CURSOR_GAP_PX = 12` away from the cursor ("any further downward movement
immediately entered the overlay itself, firing the row's onMouseLeave … and looped",
the comment records four review rounds on this exact bug).

That invariant holds only while one of the two placements fits:

```
belowTop = mouseY + 12
if (belowTop + h <= containerBottom - 4)  top = max(belowTop, containerTop)     // below
else                                      top = max(mouseY - 12 - h, containerTop) // flip above
cap = containerBottom - top - 4                                                  // max-height
```

When `h` is too big for **both** sides, the second branch's `max(…, containerTop)`
clamps `top` to the transcript's top edge, and `max-height` (`cap`) lets the panel
run nearly the full height of the transcript. The panel now spans the cursor. The
comment in `update()` says the flip "keeps the cursor strictly outside … by
construction"; that is true only until the clamp, and the comment itself names the
clamp case as "an existing edge case, not one this fix introduces" — it is exactly
the case you are hitting.

Result: cursor lands inside the panel → row `mouseleave` → panel unmounts → cursor is
over the row again → `mouseenter` → 50 ms later the panel mounts under the cursor →
repeat. That is the flicker.

### 3.2 Why "too large to fit" is the trigger

Anything that makes `h` exceed the space on both sides: a long Bash command or
heredoc, a long `Agent` prompt, a long path list; a short pane; the cursor near the
middle; a high pane zoom (the panel is zoomed with the pane, the container is not
taller for it).

### 3.3 Reproduction by simulation

I ported `update()`'s `top` / `max-height` logic into a script (`/tmp/peek-sim.mjs`,
not committed) for an 800px-tall transcript, body at 13px × 1.6, and counted the
cursor positions at which the cursor ends up inside the panel:

| Command lines | Panel height | Hover positions where the cursor is inside the panel |
|---|---|---|
| 3 | 93 px | 0 % |
| 10 | 238 px | 0 % |
| 20 | 446 px | 14 % (the middle of the pane) |
| 30 | 654 px | 66 % |
| 40+ | 862 px+ | 100 % |

So a modest 20-line command already flickers when you hover the middle of the pane.
The simulation uses my approximation of the padding/meta heights; the shape of the
result, not the exact percentages, is the point. **Not yet confirmed in a running
window** — I can do that against a fresh `task dev` if you want a screen recording.

### 3.4 Are scroll bars the answer?

Scroll bars are half of it, and the half that is not reachable today.

- The overlay already has `overflow-y: auto` and `overscroll-behavior: contain`, and
  `max-height` caps it. So a tall panel *is* scrollable in principle.
- In practice you cannot use it: reaching the scroll bar or putting the wheel over the
  panel means moving the pointer onto it, which fires the row's `mouseleave` and
  closes it. The panel is effectively non-interactive. (This is also why text in it
  cannot be selected or copied.)

So the best-practice shape is two independent fixes:

1. **Never place the panel over the pointer** (kills the flicker), and
2. **Let the pointer enter the panel** (makes the scroll bar, selection and copy
   work) with the standard *hover bridge*.

## 4. Best practice, applied here

Established pattern for a hover card that can be taller than the space (this is how
floating-ui's own hover-card recipes and most design systems do it):

1. **Choose a side by free space, then constrain the size to that side** — use
   floating-ui's `flip` (with a `bestFit` fallback) plus `size()` to set
   `max-height` to the space on the chosen side, instead of clamping the position
   and letting the box grow back over the anchor. The panel then never overlaps the
   anchor *by construction*, whatever its content.
2. **Cap to a fraction of the window, scroll inside it** — e.g.
   `max-height: min(<free space>, 60vh)` with `overflow-y: auto`. A fixed ceiling also
   keeps a 5,000-line heredoc from becoming a wall.
3. **Treat anchor ∪ panel as one hover region**, with a short close delay (≈100–150 ms)
   so crossing the gap does not close it, and `pointer-events` on the panel. Then the
   scroll bar, the wheel and text selection all work. Keep the existing "ignore enter
   while the button is down" rule.
4. **Optional: click to pin.** `UserMessageBlock` already has pin/unpin for its body
   preview; the same affordance would give a stable, scrollable reading mode for a
   long command without relying on hover at all.

For this app, point 1 has an extra, simpler form because of Ask 3: place the panel
**beside the row** (to the left or right of the pane, whichever has room), vertically
aligned near the row and clamped to the window. The cursor is over the row and the
panel is next to it — they cannot overlap, so there is no flicker regardless of height,
and the pointer can cross into the panel to scroll it. Only if neither side has room
(a pane that fills the window) fall back to above/below with a constrained height.

## 5. Ask 3 — leaving the pane

What confines it today (all in `PeekOverlay.update()`):

- `max-width: rect.width` — "so a long tool command can't escape the pane".
- `findScrollContainerRect(row)` — the transcript's scroll container, used for the
  vertical clamp and `max-height`. This is tighter than the pane (excludes the header
  and composer), not just "the pane".
- `left: rect.right` + `translateX(-100%)` — the right edge is pinned to the row's
  right edge and it grows leftward.

Nothing about the DOM confines it: it is already at `document.body`, `z-index: 50`,
`position: fixed`.

Things that must be handled when it leaves the pane:

1. **Browser-pane airspace.** Native browser panes are OS child windows that paint
   above the page's DOM. `anchored-popover.tsx` handles this with `usePaneOverlay`
   and `data-pane-overlay`; `PeekOverlay` does **not** carry `data-pane-overlay` today,
   because it never leaves its own agent pane. Once it can, a neighbouring browser
   pane would paint over it. It needs the same tag (the auto-clip service,
   `platform/pane-overlay-auto.ts`, finds `[data-pane-overlay]` by MutationObserver).
2. **Window edges, not pane edges.** The boundary becomes the window viewport (minus
   native panes), which `util/menu-position.ts` (`computeMenuPosition`: flip, shift,
   `size`, native-pane aware) already computes.
3. **Zoom.** `PeekOverlay` follows the *agent pane's* zoom (`--agent-pane-zoom`, via
   `withPaneZoom`), so the panel matches the text it describes. The new
   `AnchoredPopover` (#4208) follows *chrome* zoom (`--zoomfactor`) through a
   shell/body split and offers `zoom: "none"`. If the peek moves to `AnchoredPopover`,
   decide which zoom it should follow; I would keep the pane's, since it is a peek of
   that pane's content.
4. **Which overlay is on top.** `z-index: 50`; a panel over a neighbouring pane should
   still sit under modals and menus. No conflict found, but it should be checked when
   built.

## 6. Recommendation and options

**Option A — fix in place (smallest).** Keep `PeekOverlay`; change `update()` to
stop clamping onto the cursor (choose the larger side and set `max-height` to that
side's free space, never `max(…, containerTop)` over the cursor); add the hover
bridge; switch the line height; swap the boundary to the window. ~1 file plus tests.
Risk: it keeps a second, hand-rolled positioning engine next to `AnchoredPopover`.

**Option B — move onto `AnchoredPopover` / `computeMenuPosition` (recommended).**
`AnchoredPopover` already owns the portal, `autoUpdate` lifecycle, native-pane
airspace, and dismiss. Add a hover-card mode to it (placement `right-start` /
`left-start`, `size()` for max-height, a hover bridge), and make `PeekOverlay` a thin
wrapper that passes the row as the anchor. This removes most of `PeekOverlay`'s 376 lines of placement and
zoom code and the four-rounds-of-review edge cases in `update()`. Larger change,
touches 13 call sites' behaviour (mostly invisible for the short ones), needs the zoom
decision in §5.3.

**Option C — only the line height now.** One value; ship immediately; does not touch
the flicker.

I recommend **C immediately, then B**; A only if B is judged too big.

### Suggested order if you say go

1. Line height 1.6 → 1.4 (or `--leading-normal`), gap/padding trim. (C)
2. Fix the placement so it can never overlap the pointer, with `size()`-based
   `max-height`; add a regression test that sweeps cursor positions like §3.3 and
   asserts the cursor is never inside the panel. This test is the missing guard: the
   existing `PeekOverlay.test.tsx` does not cover the both-sides-too-small case.
3. Hover bridge + `pointer-events`, so a capped panel can be scrolled; keep the
   button-down rule.
4. Window boundary + `data-pane-overlay`, so it may leave the pane.
5. (If B) fold into `AnchoredPopover`.

## 7. Open questions for you

1. **Beside the pane or over it?** Beside (left/right) is the flicker-proof choice; if
   a pane fills the window there is no side, and it falls back to above/below with a
   capped height.
2. **Should it be interactive** (scroll bar, select/copy text, click to pin)? That is
   what makes a capped panel acceptable; it changes hover from "tooltip" to "card".
3. **A height ceiling?** I suggest `min(free space, 60vh)`; a very long command would
   scroll inside it rather than fill the screen.
4. **Zoom:** follow the agent pane's zoom (today) or the window's (`AnchoredPopover`)?

## 8. What I did not verify

- The flicker in a running window — only the placement math (§3.3).
- That every call site other than `ToolBlock` is short in practice (the other 12 look
  short from the code, but I did not audit their content).
- Behaviour with a browser pane beside the agent pane (§5.1) — reasoned from
  `pane-overlay.ts`, not exercised.

## References

- `frontend/app/view/agent/components/PeekOverlay.tsx` — placement, flip, zoom.
- `frontend/app/view/agent/hooks/useNodePeek.ts` — hover state.
- `frontend/app/view/agent/components/ToolBlock.tsx` — the tool-call peek content.
- `frontend/app/view/agent/styles/_document-nodes.scss` — `.agent-node-peek-*`.
- `frontend/app/element/anchored-popover.tsx`, `frontend/app/util/menu-position.ts`,
  `frontend/app/platform/pane-overlay.ts` — the shared popover stack (#4208).
- `docs/specs/SPEC_PEEK_PANEL_META_ROW_AND_MONO_COMMAND_2026_09_27.md`,
  `docs/specs/SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md`,
  `docs/specs/SPEC_PEEK_OVERLAY_MOUSE_Y_TRACKING_2026_09_03.md` — the specs this
  panel came from (the last one introduced the cursor-following placement).
- `docs/reports/REPORT_CHROME_ZOOM_POPOVERS_2026_10_02.md` — why `AnchoredPopover`
  splits shell and body.
