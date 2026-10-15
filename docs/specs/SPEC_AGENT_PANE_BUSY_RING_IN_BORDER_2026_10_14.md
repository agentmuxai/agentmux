# SPEC: the busy ring runs inside the pane border, as colour, not teeth

**Status:** implemented — in PR #4934 (spec and change together); the live check in §3 runs on a dev build.
**Date:** 2026-10-14
**Author:** AgentA@Area54
**Verified against:** `agentmux` `main` @ `5a8c98829`, by reading code and a
capture of a busy agent pane in the operator's 0.59.24 window (app zoom
0.85).
**Related:**
- [`SPEC_AGENT_PANE_PROGRESS_BAR_OVERLAY_NO_GAP_2026_08_25.md`](SPEC_AGENT_PANE_PROGRESS_BAR_OVERLAY_NO_GAP_2026_08_25.md): the slot overlays the pane without reserving space.
- [`SPEC_AGENT_PANE_PROGRESS_BAR_ABOVE_TAB_STRIP_2026_08_10.md`](SPEC_AGENT_PANE_PROGRESS_BAR_ABOVE_TAB_STRIP_2026_08_10.md): why the bar is portaled into the chrome.
- [`SPEC_AGENT_BUSY_ANTS_REFINEMENT_2026_06_22.md`](SPEC_AGENT_BUSY_ANTS_REFINEMENT_2026_06_22.md): the marching-ants origin.
- [`docs/retro/RETRO_AGENT_PANE_PROGRESS_RING_TOP_EDGE_GAP_2026_09_19.md`](../retro/RETRO_AGENT_PANE_PROGRESS_RING_TOP_EDGE_GAP_2026_09_19.md) and [`docs/retro/RETRO_AGENT_PANE_BUSY_RING_LEFT_EDGE_FAINT_2026_09_25.md`](../retro/RETRO_AGENT_PANE_BUSY_RING_LEFT_EDGE_FAINT_2026_09_25.md): the ring's past rounding and stacking bugs.

## 0. The ask

From the operator, 2026-10-14:

> "Let's also refine the progress border. We don't want the teeth, but we
> like the rhythm they run in. Instead of protruding teeth, the same rhythm
> should be represented with colors that operate within the border, like an
> off color different enough with broken ants like sections."

So:
- **Keep:** the rhythm, meaning the segment spacing, the speed and the direction.
- **Drop:** anything that changes the border's thickness or sticks out of it.
- **New:** the pattern is two colours *inside* the border's own band: the
  ring colour and an "off" colour, so the border reads as a marching dashed
  line.

## 1. What happens today (verified)

Two separate rings are drawn, one inside the other:

| Layer | Element | Geometry | z-index |
|---|---|---|---|
| Selection ring | `.pane-stack::after` ([`PaneChrome.scss`](../../frontend/app/element/PaneChrome.scss)) | `border: 2px solid var(--pane-ring-color, …)`, `inset: 0` | 10; 50 when focused |
| Busy band | `.pane-progress-bar-slot` (same file) holding `.agent-pane-progress-bar` ([`agent-view.scss`](../../frontend/app/view/agent/agent-view.scss)) | slot: `inset: 0` plus a **2px transparent border**; bar: `padding: 1px`, masked to that 1px ring | `calc(var(--z-pane-overlay, 4) + 1)` = 5 |

- **The pattern** is a `repeating-conic-gradient` on the bar's `::before`:
  10° of `--progress-bar-color`, then 10° of that colour mixed 25% into
  `--main-bg-color`. That's a 20° period, 18 pairs around the ring. It turns
  `rotate(20deg)` every **0.5 s**, linearly and clockwise
  (`@keyframes agent-ant-march`), on the compositor.
- **Stopping** dims the band to `opacity: 0.55`. **Idle** fades it out over 200 ms.
- **Colour:** `--progress-bar-color` follows the pane's ring colour, and the
  focused ring colour when the pane is focused.
- **"Focused alone"** (a pane with no neighbours) makes the selection ring
  transparent (`.pane-stack-focused-alone::after`), but the busy band still
  shows.

**Why it reads as teeth.** The busy band is a separate 1px line directly
inside the 2px ring. In a capture of a busy pane, the right edge shows the
solid ring and, inside it, bright and dim 1px segments alternating. Where a
bright segment sits, the border looks 3px thick. Where a dim one sits, it's
2px with a darker notch. As the pattern marches, the border's inner edge
grows and loses teeth. The rhythm is right; the thickness changing is what
reads as teeth.

## 2. Design

### 2.1 One band, the border's own

The busy pattern paints **the selection ring's own 2px band**, on top of it,
and nothing inside it:

- **Slot:** drop the 2px transparent border. The slot covers the pane
  (`inset: 0`), with the same `border-radius`.
- **Bar:** `padding` equal to the ring's width (2px, from one shared token,
  §2.5), still masked to the padding ring (`mask-composite: exclude`). Its
  outer edge is then the ring's outer edge and its inner edge the ring's inner
  edge, with the same radius on both. Border and padding at the same offsets
  from the same box round to the same device pixels, which is the property
  the 2026-09-19 retro relied on.
- **Stacking:** the slot sits just above the ring and the block mask in the
  same `.pane-stack` stacking context (`z-index` 51, above the focused 50).
  Today's 5 put it under both, which is the left-edge retro's top-ranked
  cause.
- **No scrolling:** `overflow: clip` instead of `hidden`, so the slot is not
  a scroll container and can't be offset (that retro's candidate 3).

The static ring stays where it is, under the band. If the two layers ever
round a pixel apart, the sliver that shows is the ring's base colour, the
same colour as the band's "on" sections, not a dark gap.

### 2.2 The pattern: on and off colours, same rhythm

| | Today | New |
|---|---|---|
| Period | 20° | 20° (unchanged) |
| Speed | one period per 0.5 s | unchanged |
| Direction | clockwise | unchanged |
| Duty | 10° on / 10° off | **10° on / 10° off by default** (§5 Q1 offers shorter breaks) |
| On colour | `--progress-bar-color` | unchanged: the ring's own colour |
| Off colour | ring 25% + background 75% | **ring 40% + opaque background 60%** (`--busy-ring-off`, §2.5) |
| Thickness | 1px, *inside* the 2px ring | **the 2px ring itself**; never changes |

- **The off colour** is mixed toward the pane background *without the
  window's transparency* (`--pane-bg-opaque`). `--main-bg-color` carries
  `--window-opacity` when the window is transparent or blurred, and mixed
  toward that, the off sections would composite back toward the ring below
  and lose most of their contrast (muxreview on #4934). Mixed toward the
  pane background, in every theme,
  light or dark, it reads as a *break* in the line (the "broken ants") rather
  than a second, competing colour. At 40% it differs clearly from the ring.
  Unlike today's 25% it stays visible against the background, so the pane's
  edge is never lost while the ants march.
- **Where the static ring is transparent** (focused alone), the band still
  shows both colours. A busy single pane gets a marching border where at rest
  it has none, as today.

### 2.3 States

| State | Band | Notes |
|---|---|---|
| Idle | hidden (`opacity: 0`, 200 ms fade) | The static ring shows as now. |
| Working | full on/off pattern, marching | |
| Stopping | the off colour moves toward the on colour (**ring 70%**), still marching | Lower contrast instead of today's whole-band `opacity: 0.55`. A translucent band over the ring would blend into a muddy third colour. |
| Reduced motion (`prefers-reduced-motion: reduce`) | the same broken pattern, **static** | Today it always animates. The colours still say "working". |

### 2.4 What stays the same

- The conic sweep and `rotate()` on `::before` (compositor-only animation),
  `inset: -75%`, and the solid fallback.
- `--progress-bar-color` and its focused / unfocused switch.
- The portal (`AgentProgressBar.tsx`), `role="progressbar"` and its label.
- **Conic segments still vary in length along the ring** (longer near the
  corners of a tall pane), as today. A perimeter-uniform dash (an SVG `rect`
  with `stroke-dasharray`) would fix that, but `stroke-dashoffset` animation
  runs on the main thread (§5 Q3).

### 2.5 Tokens

In [`PaneChrome.scss`](../../frontend/app/element/PaneChrome.scss) on `.pane-stack`, next to `--progress-bar-color`:

```scss
--pane-ring-width: 2px;   // the ring's border AND the busy band's padding: one value, so they can't drift
--pane-bg-opaque: rgb(from var(--main-bg-color) r g b / 1);   // the pane background, window transparency removed
--busy-ring-off: color-mix(in srgb, var(--progress-bar-color) 40%, var(--pane-bg-opaque));
--busy-ring-off-stopping: color-mix(in srgb, var(--progress-bar-color) 70%, var(--pane-bg-opaque));
```

`.pane-stack::after` uses `border: var(--pane-ring-width) solid …`, and the
bar uses `padding: var(--pane-ring-width)` over an opaque base of
`--pane-bg-opaque`, so a translucent ring colour composites the same way on
every pane.

## 3. Verification

CSS isn't testable in jsdom, so this is checked in a running app, like the
pane "+" fix (#4933). A dev build is driven over CDP with a forced
`--active` class; the method is the one in
[`RCA_PANE_TAB_PLUS_OFF_CENTER_2026_10_14.md`](../investigations/RCA_PANE_TAB_PLUS_OFF_CENTER_2026_10_14.md) §6.

1. **No teeth.** On each edge, sample the 4 device rows or columns inward
   from the pane's outer edge over a full period of animation frames. Only the
   ring's rows may ever take the on or off colour. The next row in stays the
   pane background in every frame.
2. **Both colours, in the ring.** Along each edge, the ring's rows alternate
   between the on and off colours, with segment lengths matching the 20°
   period at that distance from centre.
3. **Rhythm unchanged.** The pattern advances one period every 0.5 s (±1
   frame), clockwise, on all four edges.
4. **No gap or sliver.** At app zoom 0.85, 0.9, 1, 1.1 and device scale 1,
   1.25, 1.5, 2, there's no device row where the static ring shows through
   outside the band's on sections. Check all four edges, including the left
   edge next to a split neighbour (the 2026-09-25 retro).
5. **States:** stopping lowers contrast but keeps marching. Idle shows the
   plain ring. Focused-alone shows the band. Reduced motion is static.
6. **Themes:** the off colour is distinguishable from both the ring and the
   background in the default dark theme and one light theme.

## 4. Phases

One change: [`agent-view.scss`](../../frontend/app/view/agent/agent-view.scss) (bar, pattern, states, reduced motion) and
[`PaneChrome.scss`](../../frontend/app/element/PaneChrome.scss) (slot geometry, z-index, overflow, tokens), verified per §3.
No TypeScript changes.

## 5. Open questions

1. **Break length.** Keep 10° on / 10° off (today's exact rhythm), or make
   the breaks shorter, e.g. **14° on / 6° off**? The period and speed stay
   the same; the line just reads as mostly solid with gaps. Recommended:
   start with 10/10 and judge it live.
2. **Off colour direction.** Toward the background (a break, recommended), or
   toward white or a hue shift (a second colour)? A break keeps the ring one
   colour that the pane already owns.
3. **Uniform dash length.** Accept the conic's varying segment length (today's
   behaviour), or move to an SVG stroke with `stroke-dasharray` for equal
   dashes all the way round, at the cost of a main-thread animation?
   Recommended: keep the conic.

## 6. Docs to update

- [`agent-view.scss`](../../frontend/app/view/agent/agent-view.scss)'s long comment on `.agent-pane-progress-bar` and
  [`PaneChrome.scss`](../../frontend/app/element/PaneChrome.scss)'s on the slot, which describe the 1px inner ring.
- Public docs (`agentmux-docs`, `pane-types.md`), if they describe the busy
  ring's look. Checked with the change.
