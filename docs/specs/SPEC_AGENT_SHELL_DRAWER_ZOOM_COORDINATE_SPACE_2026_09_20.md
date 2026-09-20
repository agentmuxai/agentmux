# SPEC — Shell drawer terminal renders in a fractionally-scaled coordinate space

**Date:** 2026-09-20
**Type:** Bug diagnosis + fix proposal (diagnosis verified live via CDP; fix not yet implemented)
**Status:** proposed — diagnosis confirmed by direct measurement against a running dev
instance; no code change has shipped. Decision on the user-visible trade-off (§7) taken:
decouple.
**Scope (confirmed affected):** `frontend/app/view/agent/agent-view.tsx` (the
`zoom:` style at :2145 and the drawer mount at :2585-2594),
`frontend/app/view/agent/components/AgentShellSubblock.tsx` (`termFontSize`
compensation), `frontend/app/view/term/termwrap.ts` (`customFit`).
**Not affected:** standalone terminal panes (`term.tsx` / `term-connectelem`) — measured
at ratio 1.0, see §3.
**Related:** `docs/specs/zoom-architecture.md` (the design this violates),
`SPEC_STATUS_BAR_POPOVER_DOUBLE_ZOOM_OFFSET_2026_08_22.md` (same disease, same cure, in
the status bar), `SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10.md`,
`docs/reports/REPORT_SHELL_DRAWER_ZOOM_HISTORY_ALIGNMENT_2026_09_19.md` (investigation
log, addenda A and B).

---

## 1. Symptoms

User report, 2026-09-20:

> "no matter how high I scroll I only see the bottom half of the top line. and the link
> hover line is completely off placement."

and, narrowing it themselves:

> "the terminal pane works perfect, the terminal tab that opens in a pane is always
> aligned and zoom works perfect. but in the shell drawer, it is mostly broken."

and, correctly identifying the mechanism:

> "the outer agent pane zoom and the inner shell drawer zoom are coupled"

This is the same complaint that opened the investigation ("there is something seriously
wrong between the window it is placed in and the terminal"). It is **not** addressed by
any of the three fixes already made (drawer scrollback depth, `MuxObject` atom
reactivity, replay-before-fit ordering) — those are real but unrelated defects.

## 2. Root cause

`.agent-view` carries an inline CSS `zoom`:

```tsx
// agent-view.tsx:2145
style={{ zoom: zoomFactor(), "--agent-pane-zoom": String(zoomFactor()) }}
```

`AgentShellSubblock` is mounted **inside** that element (`agent-view.tsx:2585`) and is
handed the same factor (`agentPaneZoom={zoomFactor}`, :2594) so it can cancel the
scaling arithmetically:

```ts
// AgentShellSubblock.tsx
const termFontSize = createMemo(() => {
    const paneZoom = props.agentPaneZoom() || 1;
    return Math.max(4, Math.min(64, Math.round((BASE_FONT_SIZE * termZoom()) / paneZoom)));
});
```

The intent is stated at the mount site — *"total decoupling: the pane's zoom (this) and
the shell's own zoom (term:zoom on the sub-block) are independent controls, and neither
should visually leak into the other."* That intent is right. The mechanism is not: it
cancels the **number** while leaving the **geometry** scaled. Three consequences follow.

**2.1 The two coordinate spaces disagree.** Under CSS `zoom`, `clientWidth`/
`clientHeight` report *layout* pixels while `getBoundingClientRect()` reports *visually
scaled* pixels. `customFit` mixes them in one expression:

```ts
// termwrap.ts — customFit()
const cellWidth = core?._renderService?.dimensions?.css?.cell?.width ?? 0;  // rect-measured
const availPx = this.connectElem.clientWidth - padX;                        // layout px
dims.cols = Math.max(2, Math.floor(availPx / cellWidth));
```

**2.2 Integer font size in layout space becomes a fractional cell in visual space.**
`Math.round()` picks a whole font size, which the browser then multiplies by 0.69. Rows
no longer land on whole device pixels. Because xterm's grid is **bottom-anchored**, the
accumulated deficit surfaces at the **top** — exactly "only the bottom half of the top
line". Upstream has the same failure with the same explanation: microsoft/vscode#335328
records `22 / 1.2 * 1.2 = 22.000000000000004` inflating per-row height, yielding "54
rows instead of 57", with the missing rows appearing as "blank space above the first
line".

**2.3 The link layer is drawn into one space and displayed in another.** Measured:
backing store `445×210`, displayed `307.05×144.89`. Hit-test and underline coordinates
are computed against the backing store, so the error grows with distance from the
origin — "completely off placement".

## 3. Evidence (measured live, dev instance, CDP)

| | Shell drawer | Terminal pane |
|---|---|---|
| container | `agent-shell-subblock` | `term-connectelem` |
| nearest zoomed ancestor | **`agent-view` @ `zoom: 0.69`** | **none** |
| `clientWidth × clientHeight` (layout px) | 449 × 215 | 310 × 955 |
| `getBoundingClientRect()` (visual px) | 310 × **148.36** | 310 × **955** |
| ratio visual/layout | **0.6904** | **1** |
| `.xterm-screen` style | `445px × 210px` | `306px × 954px` |
| link canvas backing → displayed | `445×210` → **`307.05×144.89`** | `306×954` → **`306×954`** |

The pane is 1:1 in every column. Same terminal code, same handlers; only the coordinate
space differs. That is the whole difference between "works perfect" and "mostly broken".

## 4. Upstream corroboration

xterm.js does not support being rendered inside a CSS-scaled subtree:

- xtermjs/xterm.js#2584 — *Selection doesn't respect the zoom CSS rule* (open; labelled
  `good first issue` / `help wanted` / `type/bug`).
- xtermjs/xterm.js#3242 — *Character selection is affected by CSS scaling transforms*
  (the `transform: scale()` variant of the same bug — so switching `zoom` for
  `transform` would not help).
- xtermjs/xterm.js#2662 — *Renderer is blurry when window zoom level is changed*.
- xtermjs/xterm.js#4113 — FitAddon's computed size disagrees with element resizing when
  `devicePixelRatio !== 1`, causing extra resize adjustments.
- microsoft/vscode#335328 — float error scaling `charHeight` to device pixels; fixed by
  subtracting a tolerance before `ceil`.

Conclusion: this class of bug is not fixable *inside* xterm's consumer by tuning
numbers. The terminal must render in an unscaled space.

## 5. This violates the project's own documented design

`docs/specs/zoom-architecture.md` states the intended split:

> **Per-pane zoom** — "scales terminal font size via block metadata (`term:zoom`) …
> This is the primary zoom mechanism users interact with."
> **Chrome zoom** — `--zoomfactor` CSS custom property — "a cosmetic feature".

and records that compensator-style counter-scaling was deliberately removed:

> "`--zoomfactor` and `--zoomfactor-inv` were *compensators* … That system has been
> fully removed."

So: terminals are supposed to scale **by font size**, and the codebase has already
rejected inverse-scaling compensators as an architecture. The drawer currently does
both of the things that design forbids.

There is also direct precedent for the cure.
`SPEC_STATUS_BAR_POPOVER_DOUBLE_ZOOM_OFFSET_2026_08_22.md` (Resolved, PR #2736) fixed
"offset (and undersized)" popovers by removing the self-applied `zoom:`; five statusbar
files now carry `// Deliberately NOT zoom: var(--zoomfactor)`. And `agent-view.scss:350`
documents the marching-ants progress bar being moved so that "no zoom-compensation math
needed here … outside `.agent-view`'s per-pane `zoom` CSS property entirely, alongside
the tab strip and other unzoomed pane chrome."

## 6. Options considered

**Option A — take the drawer out of the zoomed subtree. (recommended)**
Render the shell drawer in the unzoomed pane-chrome tree, as the progress bar already
does, and delete the `/paneZoom` division from `termFontSize`. The terminal then lives
at ratio 1.0 exactly like a terminal pane: whole-pixel rows, 1:1 link canvas, crisp
glyphs. Cost: a structural move — the drawer is currently nested composer-strip →
`ResizableDetailsDrawer` → subblock, so its slot/positioning must be re-established
outside `.agent-view`.

**Option B — counter-zoom the drawer subtree** with
`zoom: calc(1 / var(--agent-pane-zoom))`. Measured live: this *does* restore ratio 1.0
and a 1:1 link canvas (`310×140` backing displayed at `310×140`) at factor 0.69, so it
is not theoretically dead. Rejected anyway: it is precisely the `--zoomfactor-inv`
compensator pattern §5 records as "fully removed", it re-derives correctness from a
floating-point round trip (`0.69 × 1/0.69`) that is only verified at one factor, and it
leaves the terminal's correctness dependent on a CSS `calc` staying exact at every
future zoom step and DPI. Cheap to try, expensive to trust.

**Option C — restrict pane zoom to factors yielding integer cell sizes.** Keeps the
visual coupling. Rejected: depends on font metrics, breaks on font change, DPI change,
or a new zoom step; reduces misalignment rather than eliminating it.

**Option D — scale the drawer by font size only, no CSS zoom on it, but keep it inside
`.agent-view`.** Not possible: CSS `zoom` on an ancestor scales the whole subtree, and a
descendant cannot opt out except by counter-scaling (Option B).

## 7. Decision and its user-visible consequence

**Option A.** Confirmed with the user 2026-09-20.

Consequence, accepted explicitly: **the shell drawer no longer scales when the agent
pane is zoomed.** Its size is governed solely by its own `term:zoom` (Ctrl+Wheel over
the drawer), exactly like a terminal pane. This is the behaviour the mount-site comment
already claims to want ("neither should visually leak into the other"); today the pane's
zoom does leak into the drawer, just badly.

## 8. Implementation plan

1. Move the drawer's render site out of the `zoom`-bearing `.agent-view` element into
   the unzoomed pane-chrome tree, following the marching-ants precedent
   (`agent-view.scss:350-353`). Preserve `ResizableDetailsDrawer` behaviour (drag
   height, persisted `term:shellheight`).
2. Delete the `/ paneZoom` division in `AgentShellSubblock.tsx`'s `termFontSize`, and
   drop the now-unused `agentPaneZoom` prop (and its plumbing at `agent-view.tsx:2594`).
3. Leave `customFit`'s `clientWidth`-based width reclaim as-is — it is correct once the
   element is no longer in a scaled subtree. Do **not** "fix" it by switching to
   `getBoundingClientRect`, which would paper over the nesting instead of removing it.
4. Add the `// Deliberately NOT under .agent-view's per-pane zoom` comment at the new
   site, matching the five statusbar precedents, so this is not silently re-nested.
5. Declare the `agent-pane` container on **both** `.agent-view` and the new
   `.agent-view-zoomed` wrapper — not on the wrapper alone.

   As implemented, per-pane `zoom` moved onto a wrapper holding everything that
   should scale, and the container was moved with it so `@container agent-pane`
   breakpoints kept firing against the same *visual* width. That is correct for the
   presentation view, and wrong everywhere else: `AgentPicker.tsx:957,971` and
   `AgentHistoryTabView.tsx:56` render a **bare** `.agent-view` with `zoom` applied
   directly and no wrapper. Declaring the container only on the wrapper silently
   stopped every `@container agent-pane` rule (`_responsive.scss`,
   `_shell-node.scss`, `_composer-strip.scss`) from matching in the History tab and
   the Picker — compact/narrow-pane layouts and hide-on-narrow elements never
   triggered there again.

   Declaring it on both is safe because container queries resolve to the NEAREST
   matching ancestor: inside the presentation view the wrapper wins; for the bare
   call sites `.agent-view` serves. Caught by review on #3456, not by any test —
   see §9.

6. Move `.agent-pane-loading-overlay` OUT of the wrapper, keeping it a direct child
   of `.agent-view`. It is `position: absolute; inset: 0`, so it covers its nearest
   *positioned* ancestor; the wrapper is `position: relative`, so nesting the overlay
   inside it shrank the cover and left an open Shell drawer visible for the whole
   load. Keeping it outside also keeps it unscaled.
   See `REPORT_AGENT_PANE_LOADING_UI_2026_09_20.md` §F.

## 9. Acceptance criteria

Measured on a running instance, drawer open, at pane zoom ≠ 1 (e.g. 0.69):

- `connectElem.getBoundingClientRect().width / connectElem.clientWidth === 1`.
- Link-layer canvas backing dimensions equal its displayed dimensions.
- Scrolling to the top of scrollback shows a **whole** first line, not a partial row.
- Hovering a file path underlines that path, with the underline aligned at both the
  left and right edges of the pane (the error was distance-dependent, so check both).
- Ctrl+Wheel over the drawer resizes only the drawer; Ctrl+Wheel over the agent pane
  resizes the pane and leaves the drawer's cell size unchanged.
- Regression: repeat with pane zoom at 1.0 and confirm nothing changed there.
- **Every `.agent-view` render site keeps a matching `agent-pane` container.** Narrow
  the Agent Picker and the History tab below the `@container agent-pane` breakpoints
  (e.g. 260px / 249px) and confirm the compact layouts still trigger. These surfaces
  render a bare `.agent-view` with no wrapper, so a container declared only on the
  wrapper leaves them permanently at the wide layout — silently, with no error and no
  failing test. This was missed in the first implementation and caught by review;
  a container-query assertion covering all three call sites would have caught it.

## 10. Risks

- The move is structural; layout regressions in the composer strip / drawer resize
  handle are the main risk, not terminal behaviour.
- Anything else relying on the drawer being inside `.agent-view` for styling
  inheritance (theme vars, fonts) must be re-checked at the new site.
- `--agent-pane-zoom` remains published for other consumers; only the drawer stops
  consuming it. Verify no other component reads it expecting the drawer to be scaled.
