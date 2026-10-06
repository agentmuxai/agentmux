# Report: the tool-call hover panel — line height, flicker when it is too tall, and leaving the pane

**Status:** active: Option A (§6) implemented in #4215 (placement, hover bridge, window boundary, line height); Option B (move onto `AnchoredPopover`) remains open; overshoot limited to a readable width (§9)

Date: 2026-10-02 · Author: Loap · Base: `main` @ `f5dfeafbc` (v0.59.5)

Written as analysis, then Option A was implemented in #4215 (the report's own
files, `PeekOverlay.tsx` and the new `peek-placement.ts`, are what that PR
changes). The flicker is reproduced by a simulation of the placement code (§3.3),
not by watching the app; neither the report nor #4215 has seen it in a running
window.

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

I recommended **C immediately, then B**; A only if B is judged too big.

**What shipped (#4215): Option A**, which includes C. B was left for a follow-up
because it needs the zoom decision in §5.3 and touches how all 13 call sites are
anchored.

**Revision after first use.** §4's "beside the pane" placement shipped first and was
changed on review. Its problem: a command short enough to fit above or below the
pointer stayed on the pane, and a taller one jumped to the side of the pane, so the
same hover behaved two ways depending on length. The panel now always stays near the
pointer, like every other peek:

- **Wider than the row:** pinned to the row's left edge, extending right over the
  pane border up to the window edge. Otherwise right-aligned to the row as before.
- **Taller than the transcript:** it leaves the transcript, above or below the
  pointer (whichever has more room), and its height is cut to that room so it can
  never reach the pointer; the rest scrolls. This is still the flicker fix: the
  cause was the panel covering the pointer, not the panel being near it.
- The 60%-of-window height ceiling is gone: it uses all the room on its side. A very
  tall command therefore shows about half the window before scrolling, not the
  whole of it, because the other half is on the other side of the pointer.

§7's "beside or over" question is therefore answered as "near the pointer".

**Revision 2 (2026-10-03): a tall panel could not be entered.** Found in use: some tall
panels with a scroll bar could never be reached. Two causes compounded:

1. *The panel followed the pointer.* Every move over the row re-placed it at pointer + 12px,
   with its height cut to the room left, so a pointer heading for it pushed it away and its
   scroll bar shrank as it went.
2. *The way out crossed the next row.* Leaving the row meant crossing up to 12px of the next
   tool row before reaching the panel, and that row's own peek opened (after its 50 ms enter
   delay) on top of this one, before the 150 ms linger could help.

Fix, for panels that scroll (enterable) only; a panel that fits behaves as before:

- **Pinned** once it turns out to scroll: it stops following the pointer (it still moves with
  its row if the transcript scrolls).
- **Flush with the row**: its near edge is `min(pointer + 12, row bottom)` (or the mirror
  above), so no other row lies between them (`computePeekVertical({ flushToRow })`).
- **Approach-aware linger**: the 150 ms grace is re-armed while the pointer keeps getting
  closer to the panel, up to 1 s, so a slow or diagonal approach still arrives.
- **Other peeks wait**: while a panel is being crossed to (or the pointer is on it), other
  rows' peeks hold off (`peek-bridge.ts`); if the pointer never arrives, a still-hovered row
  opens its peek when the grace ends.

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

## 9. Decision (2026-10-06): the overshoot only reaches for a readable width

**Owner's goal.** Stop wide panes from getting hover panels that reach far past them. Before this, a wide panel (one wider than its row) could overshoot the row by half the row's width in total, whatever the pane's width (`MAX_OVERSHOOT_FRACTION`, #4350, split left/right by the pane's place in the window). So a 1200px pane could get an 1800px panel.

**Rule.**

```
overshoot = min( MAX_OVERSHOOT_FRACTION × rowWidth,     // 50%: unchanged for thin panes
                 max(0, readableWidth − rowWidth) )     // never past a readable width
readableWidth = READABLE_WIDTH_CHARS (120) × 1ch of the panel's code font (--font-mono)
```

The overshoot is then split left/right by the pane's position exactly as before (#4350), and window edges still hand room from one side to the other. The 600px minimum for tall panels (#4267) still applies, capped to the room.

At the default font, 120ch is about 950px. The total overshoot past the pane is then:

| Pane width | Panel can reach | Overshoot (total) |
|---|---|---|
| 350px (pane minimum) | 525px | 175px (50%) |
| 500px | 750px | 250px (50%) |
| 700px | 950px | 250px (36%) |
| 900px | 950px | 50px (6%) |
| 950px or more | the pane | 0 |

The most any panel ever reaches past its pane is about 317px (950 / 3), for a pane of about 633px, where half the pane equals the gap to the readable width. The rule does not depend on window or monitor size.

**Why this rule.** No standard covers "how far may a hover panel leave its container". Three established practices point the same way:

1. **Cap popover width by what is readable, not by the trigger.** Simple tooltips use a fixed px max-width (about 200–320px; Bootstrap hard-codes 276px). Sizing to the trigger is for select-style menus.
2. **VS Code's content hover, the closest analogue (rich, code-heavy):** max width = `max(66% of the editor width, 750px)`, never beyond the window (minus 14px); the user can drag it larger. A narrow editor's hover spills past the editor to reach a readable floor; a wide editor's never does. That is the same shape as this rule.
3. **Line length.** Prose reads best at 50–75 characters (WCAG 1.4.8: at most 80). Code style guides settle on 80–120 characters (PEP 8: 79, Black: 88, Google: 100, GDS: 120), and longer lines are known to be harder to read. Past about 120 characters, more width mostly adds blank space or lines too long to track. So 120 is the upper end of accepted code widths: generous for tool output, which is code and logs.

**Alternatives considered and rejected.**

- **Interpolate by the pane's share of the window** (50% at the 350px minimum, 0% at full window width; the owner's first proposal). It depends on the monitor: on a 3840px window a 1500px pane still overshoots about 33%, or 500px. It also limits the *share*, while what the eye notices is the *distance*: share × width peaks around 300px for mid panes on a 2000px window, and grows on larger ones. Its two anchor points (350px gets 50%, a half-window pane gets 25%) also cannot both hold on one straight line.
- **Interpolate by an absolute pane width** (50% at 350px, 0% at, say, 1200px). Monitor-independent, and close to the chosen rule in effect: it peaks around 210px at a 600px pane. But its end point is arbitrary, while the chosen rule's cap is tied to a reason (readable line length) and to the panel's real font.
- **A fixed px cap on overshoot** (e.g. at most 250px). Simple, but wide panes would still overshoot when they don't need to.

**Implementation.**

- `frontend/app/view/agent/components/peek-placement.ts`: `READABLE_WIDTH_CHARS`, the `readableWidth` input of `computePeekHorizontal`, and the `min(…)` above. It is pure geometry, swept by `peek-placement.test.ts` (the "readable width" block pins the table; the invariant sweep includes it).
- `PeekOverlay.tsx` `measureReadableWidth()`: measures `120ch` in `var(--font-mono)` inside the panel, so it follows the pane's zoom and font size. If nothing can be measured (0), the limit is skipped and the 50% cap alone applies (the previous behaviour).

**To reassess.**

- Tune the single constant `READABLE_WIDTH_CHARS`: 100 is stricter, 140 looser.
- `MAX_OVERSHOOT_FRACTION` still sets the thin-pane end.
- If users ask for wider panels for very long lines, VS Code's answer was a resizable hover (vscode#185751), not a larger default.
- Not verified in a running window: the measured 120ch at real fonts and zoom levels. Check the panel on a 350px, a 700px and a 1200px pane.

**Sources.** UXPin, [What is a tooltip](https://www.uxpin.com/studio/blog/what-is-a-tooltip-in-ui-ux/) and [Optimal line length](https://www.uxpin.com/studio/blog/optimal-line-length-for-readability/); [Bootstrap-Vue popover](https://bootstrap-vue.org/docs/components/popover/); [VS Code `contentHoverWidget.ts`](https://github.com/microsoft/vscode/blob/main/src/vs/editor/contrib/hover/browser/contentHoverWidget.ts), [vscode#14165](https://github.com/microsoft/vscode/issues/14165), [vscode#185751](https://github.com/microsoft/vscode/issues/185751); [Baymard, line length](https://baymard.com/blog/line-length-readability); [Wikipedia, Line length](https://en.wikipedia.org/wiki/Line_length); [PEP 8](https://peps.python.org/pep-0008/); [Real Python on PEP 8](https://realpython.com/python-pep8/); [GDS Way, Python style](https://gds-way.digital.cabinet-office.gov.uk/manuals/programming-languages/python/python.html).

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
