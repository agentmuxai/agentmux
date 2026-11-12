# REPORT: Closing a pane: what should move, how fast, and with what curve

- **Date:** 2026-11-11
- **Author:** Camper@narko
- **Task:** #29004 (research). The design it leads to is [`SPEC_PANE_REFLOW_GLIDE_2026_11_01.md`](../specs/SPEC_PANE_REFLOW_GLIDE_2026_11_01.md) (#15458, revised the same day); the build is #15459.
- **Read against:** `origin/main` 63848b092. Nothing was run or measured for this report (§7 proposes one check).

The operator's words (dogfooding 0.60.20): "when closing panes, is there a best practice animation? currently its immediate, but research the latest best practices, we want it quick and snappy, but still smooth. currently when I close a pane in agentmux, it is immediate, no animation at all".

## 1. The answer

**Yes, animate it, as one gesture of 160 ms.** At the frame the pane closes, two things start together:

1. **The closed pane fades out in place** over 100 ms (opacity only, a ghost of its frame, not its content).
2. **The other panes glide into the space** over 160 ms with an ease-out curve: the motion a pane drag already uses on main.

They overlap; nothing waits for the fade. The layout is already final at the first frame (focus, terminal sizes, the message to srv), so the motion is only how the change is drawn and a close can never lag the click. With Animations Off it is today's instant close.

Both timings are tokens AgentMux already has (`--duration-fast` 100 ms, `--duration-base` 160 ms, `ease-out`), and the glide is the code path of the pane drag's live reflow, so the build adds a trigger and a ghost, not a motion system.

## 2. Where AgentMux is today

- A close is instant: every other pane takes its new rectangle in one frame, and nothing shows where the closed pane's space went.
- Since this week a pane **drag** does move the other panes smoothly (the live reflow, on by default since 2026-11-11): each pane is laid out once at its real new size and only its four edges travel, as a place and a clip, 160 ms ease-out. When a dragged pane rests over its own slot, the layout already shows a close: the pane fades and the others close over it.
- The motion policy is the app's own (`shouldReduceMotion`, the Animations speed), not the OS query. The drag's glide is "essential spatial motion": under Reduce movement it shortens to 100 ms; at speed 0 (Off) there is none.

## 3. What others do

Checked against the source or the official tokens on 2026-11-11, unless the row says otherwise.

| Who | A close | The neighbours | Source |
|---|---|---|---|
| **Chromium's tab strip** | One animation state per tab, interpolated from where it is to its target | The same animation: **200 ms, ease-out** (`kAnimationDuration`, `gfx::Tween::EASE_OUT`) | `chrome/browser/ui/views/tabs/tab_animation.h` and `.cc` (read at tag 76; current main not reachable) |
| **GNOME Shell** (a window) | **150 ms**, opacity to 0 and scale to 0.8, ease-out-quad; a dialog 100 ms | not tiled | `js/ui/windowManager.js`: `DESTROY_WINDOW_ANIMATION_TIME = 150` |
| **niri** (scrolling tiling compositor) | **150 ms, ease-out-quad** | a critically damped spring (damping 1.0, stiffness 800): no overshoot | niri wiki, Configuration: Animations |
| **Hyprland** (tiling compositor) | about **150 ms**, linear, shrinking to 87% with a fade (`windowsOut 1.49`, `fadeOut 1.46`; the unit is 100 ms) | about 480 ms, ease-out-quint (`windows 4.79`) | the unit is from the Hyprland wiki; these three lines are from a copy of the example config, not the official file |
| **Material 3** | tokens: short3 150 ms, short4 200 ms; accelerate curves for leaving (`emphasized-accelerate` 0.3, 0, 0.8, 0.15) | decelerate curves for arriving (0.05, 0.7, 0.1, 1) | `material-web` motion tokens. That exits are shorter than entrances is its guidance as summarised, not quoted |
| **Fluent 2** | tokens: fast 150 ms, normal 200 ms; `curveAccelerateMid/Max` | `curveDecelerateMid/Max` | `fluentui` tokens `durations.ts`, `curves.ts` |
| **Apple** | No number published. "Aim for brevity and precision in feedback animations"; "generally avoid adding motion to UI interactions that occur frequently"; "don't make people wait for an animation to complete before they can do anything" | | Human Interface Guidelines, Motion |
| **Nielsen Norman Group** | "the duration of most animations should be in the range of 100–500 ms"; things appearing take slightly longer than things leaving (its example: 300 ms in, 200 to 250 ms out); 0.1 s is the limit for feeling instant | | nngroup.com, animation duration; response times |
| **VS Code** | No close animation for a tab or an editor group was found in its list of transitions (an inference from issue #175990, not a statement) | | not verified |
| Zed, WezTerm, Ghostty, Zellij, tmux, iTerm2, Warp, Raycast, Safari, Arc | no primary source found | | not verified |

Apple's second sentence matters here: closing panes is frequent, so the motion has to be short enough not to be noticed as a wait.

## 4. What they agree on

1. **A close is short.** The exits above sit between 100 and 200 ms. That is under the length where a person waits for the UI, and long enough for the eye to follow where things went.
2. **The leaver and the neighbours move at the same time.** Nobody plays the exit and then the reflow: a sequence doubles the length and reads as two events. The neighbours start on the first frame.
3. **The leaver does little.** A fade, sometimes with a slight shrink. It is going away, so it gets the shorter, simpler motion; the eye should follow the panes that stay.
4. **Arriving decelerates, everywhere.** The neighbours are arriving at their new place, so they ease out (fast start, soft landing), which is also what makes a short motion feel quick: most of the distance is covered in the first half. For the leaver the sources differ: the design systems say accelerate, the compositors ease out or run linear. On a 100 ms fade of opacity the curve is barely visible, so the existing exit preset is kept.
5. **No overshoot on large surfaces.** Springs and bounces are for small things; a pane that overshoots covers its neighbour.
6. **Cheap properties only.** Opacity and transform (and a clip) are drawn without laying the content out again. Animating a size lays everything out every frame, which is what made the May 2026 pane animation heavy enough to be removed.
7. **Interruptible and never blocking.** A second close during the motion starts from where things are; the real work of the close does not wait for the picture.
8. **Reduced motion is honoured,** by shortening or removing the motion.

## 5. The recommendation in detail

| | What | Property | Length | Curve |
|---|---|---|---|---|
| The closed pane | a ghost of its frame (background, border, header strip) at its last rectangle | `opacity` 1 to 0 | 100 ms (`--duration-fast`) | `ease-out` (`MOTION.exit`) |
| The other panes | each edge travels from its old place to its new one; a pane that grows is laid out at its new size at once and uncovered | `transform` and `clip-path`, read from four registered edge properties | 160 ms (`--duration-base`) | `ease-out` |
| Sizes, focus, terminal fit, srv | at the first frame, as today | none | 0 | none |

- **Overlap, not sequence.** Both start on the frame the tree changes. The fade is done before the neighbours land, so the last third of the glide is over clean background, never over a half-faded pane.
- **Why a ghost and not the pane itself.** The pane's content is torn down at once (see §6); a copy of a terminal's canvas is blank and a copy of a long transcript is heavy. Its frame is enough to say "this went away here".
- **Why not scale the leaver down.** A shrink toward the centre pulls the eye to the pane that is leaving, and on a pane that fills half the window it is a large moving surface. Fade only. (An option if the operator wants more: scale to 0.98, the same 100 ms.)
- **Why 160 and not 120 or 240.** 120 ms is at the edge of reading as a jump on a large surface; 240 ms starts to feel like waiting when closing several panes in a row. 160 ms is the drag's timing, which the operator has used and approved, so a close, a drop and a drag feel like one thing.

## 6. Edge cases, and the teardown

| Case | What happens |
|---|---|
| The last pane of a tab | Nothing glides (no neighbour). The ghost fades over the empty tab's placeholder. |
| Closing during a pane drag or a tear-off | The drag's reflow is already showing a layout; the close changes its targets and each edge carries on from where it is drawn. |
| Closing several panes quickly | Each close replaces the targets; edges retarget in flight, no queue builds, and the total is never longer than one glide after the last close. |
| A browser pane | A native view cannot be moved by a transform. It takes its new rectangle at once while the other panes glide, as in a drag today; it joins the glide when the drag's browser phase lands. A closed browser pane's native view is gone at the first frame, so its ghost shows the frame only. |
| A hibernated pane | It is a DOM pane like any other. |
| Reduce movement | The glide stays, at 100 ms, and the ghost fades at the same length. |
| Animations Off (speed 0) | No motion and no ghost: today's close. |
| A hidden window or tab | No motion; the layout is simply final when it is shown. |
| More than 24 panes | The change snaps (the drag's cap). |

**Teardown does not wait.** The pane's block, its terminal or agent view and its process handling are released exactly when they are today; the ghost is a new, empty element that removes itself. Nothing in the close reads the animation's end, so a stalled or skipped animation cannot hold a pane's resources or delay the next action.

## 7. Risks, and one check proposed

- **The first frame of a close with several terminals growing at once** is today's cost (each fits once). The glide adds no layout work, but it makes a slow first frame more visible than a jump does. Proposed, not run: one trace of a close in a tab of four terminals with long buffers, in my own test build, before the build's PR merges. It is a single instrumented close, not a timed lab; I will ask before running anything longer.
- **Beside a browser pane** there can be a bare strip, or the browser pane over a neighbour's moving edge, for up to 160 ms.
- **A growing pane is uncovered, not slid.** The same as in a drag.

## 8. What the operator decides

The five decisions are in the spec's §0, each with a recommendation. In short: animate every tree change through the one trigger (close first), with the drag's feel, a fading ghost for the closed pane, browser panes jumping for now, and Reduce movement shortening the glide rather than removing it.
