# Spec: A working agent shows a progress line on its pane tab pill, in every pane; a collapsed pane never shows the border animation

**Status:** implemented — approved by the operator on 2026-10-25 with the recommendations in §9; built in task #2360 (spec and change together). The live check in §5 runs on a dev build.
**Date:** 2026-10-24
**Author:** Camper@narko
**Task:** Spec #2359; the build is #2360 and starts after this is approved
**Requested by:** the operator, through AgentY, with three points from AgentX (drive the line from the existing agent state, decide the border explicitly, use the app's motion policy); reviewed by AgentX, whose five changes are folded in
**Verified against:** `agentmux` `main` @ `d5e74791d` (v0.60.0, 2026-10-24), by reading the code named below; re-checked against `55950215a` (2026-10-25), where the pane tab, pane chrome, agent-state and motion code is unchanged except Kanban card name colours. Nothing here has been run; the two behaviours marked "to confirm live" in §2 and §5 are inferred from reading.
**Builds on:** [SPEC_AGENT_PANE_BUSY_RING_IN_BORDER_2026_10_14.md](SPEC_AGENT_PANE_BUSY_RING_IN_BORDER_2026_10_14.md) (the border animation), [SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md](SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md) §7.1 (which left a persistent busy signal for its own spec: this one), [SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_2026_10_15.md](SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_2026_10_15.md) (the motion policy), [SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md](SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md) (the `limited` and `signed_out` states)

## 1. Ask

The operator, 2026-10-24:

> "lets also refine how progress indicators work. we need to know if an agent is in progress even if its pane tab isn't selected. I think one solution is add a progress animated line to the top of the pane tab pill for that agent. Also, if a pane is minimized, then it doesn't matter what pane tab you select, you never see the border animation, only the new pane tab mini animations."

So:

1. A thin animated line along the top of the pane tab pill of every working agent, selected or not, in every pane (expanded, collapsed, floating, in any window).
2. A collapsed pane never shows the border animation, whichever tab is selected. Only the pill lines show there.
3. Say what an expanded pane shows for its selected tab, so the two don't double up.

## 2. What happens today

| Where | What it shows for a working agent | Source |
|---|---|---|
| **The pane border** | A marching two-colour band in the 2px ring (`.agent-pane-progress-bar`, ring colour on / 40% off, 0.5 s per 20°, a `rotate` on the compositor). Idle fades out in 200 ms. | `AgentProgressBar.tsx` portals into `.pane-progress-bar-slot` (`PaneChrome.tsx`); `agent-view.scss`; driven by `paneBusy()` in `hooks/useWorkingIndicator.ts` |
| **Which tab's agent** | Only the **selected** tab's. `PaneChrome` hands the slot to the active view model (`setProgressBarMount`) and takes it back on a switch, so a working agent behind another pill rings nothing. | `PaneChrome.tsx` slot handoff; `PaneChrome.test.tsx` "slot handoff" |
| **A collapsed pane** | The slot is `abs-fill` on `.pane-stack`, and the collapse rule (`.tile-minimized .block-content { visibility: hidden }`) hides only the content, so the border keeps marching around the chip. **To confirm live:** read from the CSS, not seen. This is what the operator describes. | `tilelayout.scss`; no `minimized` check in `AgentProgressBar`, `PaneChrome` or the SCSS |
| **The pill** | Nothing about work. The activity flash (`.pane-tab::before`) pulses with a tool-call sound; `UnseenTurnsDot` marks a finished turn nobody has seen; the active pill has a bottom underline. No working, needs-you, limited or signed-out mark. | `PaneTabStrip.tsx` / `.scss`; `UnseenTurnsDot.tsx` |
| **The window tab** (workspace tab bar) | Only the same flash. `agentActivity.ts` keeps `busyPanes`, but nothing in the UI reads it. | `tab.tsx`; `agentActivity.ts` |
| **Swarm** | A status chip per agent: working (`--error-color`), question (`--accent-color`), idle (`--success-color`), out of quota or signed out (`--warning-color`). | `swarm-status-chip.tsx`, `swarm-view.scss` |

The gap the operator names: an agent working behind an unselected pill, or in a collapsed pane, is invisible unless the Swarm is open. The flash only says "a tool just ran", for a moment.

## 3. Design

### 3.1 The line

A 2px line along the top of the pill, inside it (`overflow: hidden` on `.pane-tab` already clips it). It has a faint **track** (the line's colour at 25%, full width) and, while working, a **bar** about 40% of the pill wide that sweeps left to right and starts over: an indeterminate progress bar, since there is no real percentage. It is the same shape of animation as the compaction bar (`agent-compaction-indeterminate`, `translateX(-100% → 300%)`), moved into the shared keyframes (§6) so it has one definition.

- **Placement.** `top: var(--pane-ring-width)`, not `top: 0`. The pane's ring (z 50) and the busy band (z 51) overlay the top 2px of the pane, and the tab strip is flush with it, so a line at `top: 0` would sit under an opaque ring. **To confirm live** in the first dev build.
- **Element.** A real child of the pill (`<span class="pane-tab-progress" aria-hidden="true">`), not a pseudo-element: `::before` is the activity flash, and a child is testable. It sits above the flash overlay (`z-index: var(--z-behind)`) and below the icon, label and close button, which don't reach the top 2px.
- **Sweep.** `transform: translateX()` only, on a bar with `will-change: transform` while working. The bar's ends fade (`linear-gradient(90deg, transparent, colour, transparent)`) so there is no hard edge crossing the pill. One pass is **1.6 s**, as the compaction bar's.

### 3.2 States

The line reads the agent's state as srv decides it (`agent_state.rs`: working, waiting, limited, signed_out; idle and the rest show nothing). Every look has its own **shape**, so none depends on colour alone (WCAG 1.4.1): a moving bar, a solid line, a dotted line, a dashed line. The pill's accessible description says it in words too.

| Agent state | Line | Colour |
|---|---|---|
| **Working** (a turn runs, no question pending) | Track plus the sweeping bar | The pill's own colour: `--pane-tab-underline`, else `--accent-color` (the colour the activity flash and the active underline already use) |
| **Waiting** (a turn runs and a question waits for the user) | Full-width **solid** line, still | `--accent-color`, the Swarm question chip's colour |
| **Limited** (out of quota until a reset) | Full-width **dotted** line, still | `--warning-color`, the Swarm unavailable chip's colour |
| **Signed out** | Full-width **dashed** line, still | `--warning-color` |
| Idle, stopped, error, a terminal pane | Nothing | |

There is no real progress to show, so none is invented: no fill that grows, no percentage.

The state is also said in words, in two places, with the Swarm chip's wording ("Working", "Waiting for you", "Out of quota until Sun 7 PM", "Signed out"): a line in the pill's tooltip (the existing `getTooltipDetail`), and the pill's **accessible description** (`aria-describedby` to a visually hidden element inside the pill), because a `title`-style tooltip is not reliably read by screen readers. The pill is also `aria-busy="true"` while working. Nothing is announced by a live region: a change every few minutes per agent would be noise.

### 3.3 Where the state comes from

The frontend never receives the string `agent_state`; for local agents it receives the same inputs srv's `decide()` reads, and already derives the same answer from them:

- `controllerstatus` events, scoped to a block (`turn_active`, `is_agent_pane`), pushed to every window with a subscription;
- the block's meta: `term:awaiting_user` and `agent:last_failure`;
- `rankAgentStatus` (`store/agent-status.ts`) and `unavailableOfMeta` (`store/agent-unavailable.ts`), the two functions the Swarm line and chip already share.

The build adds one store, `store/pane-tab-progress.ts`: `paneTabProgress(blockId)` returns `"working" | "waiting" | "limited" | "signed_out" | null`, built from those inputs for the blocks that have a pill in this window. One subscription per such block, opened when the pill mounts and closed when it goes; the first value comes from `BlockService.GetControllerStatus`, as the Swarm seeds its own. That is the existing signal, not a new one: no srv change, no new event.

**One table, two tests.** The rows live in one shared JSON fixture (`test/fixtures/agent-state-decide.json`: controller type, `turn_active`, process status, exit code, `awaiting_user`, failure class and reset, `now`, and the expected state). A Rust test in `agent_state.rs` and `pane-tab-progress.test.ts` both read it, so a change to `decide()` that the frontend rule doesn't follow fails CI instead of drifting. Manoz owns `agent_state.rs` and is told in the build task.

**Limited ends by the clock, with no event.** `decide()` turns `limited` into `idle` once the reset passes, and nothing publishes at that moment. The store therefore schedules a re-evaluation at `resetsAtMs` for each pill that is limited (one timer per such pill, cleared when it changes or goes), so the amber line goes at the reset and not at the next unrelated change. The shared fixture includes a `limited` row evaluated at `now` and at `resets + 1`, and a store test uses fake timers for the same.

**Clearing a signed-out agent** after a re-sign-in (#5421) reaches the frontend as the `agent:last_failure` meta being cleared, which the store already watches, so no extra signal is needed.

**Held reasons are not shown on a background pill.** `launching`, `compacting`, `reconnecting` and `stopping` are facts of a mounted agent view, not of the block. A background agent in one of them shows its `turn_active`: working while compacting (the turn is open), nothing while launching. The selected tab's own border and working row keep showing them.

### 3.4 The border: expanded and collapsed (decision D2)

**Recommendation: the border stays, for the selected tab of an expanded pane; the line is everywhere else; one agent never shows both.**

| Pane | Selected pill | Other pills |
|---|---|---|
| Expanded | **Border** marches (as today). **No line.** | Line, per §3.2 |
| Collapsed | **Line.** No border. | Line |

- In an expanded pane the selected agent's transcript is open: its border, working row and question card already say everything, and a line above the very title the border runs around would say it twice. The line is for what you cannot see.
- In a collapsed pane there is no transcript in view, and the operator asked for the border to go: the line is all there is, on the selected pill too.
- **What an expanded pane shows for its selected tab when that agent is waiting, limited or signed out: no line and no border.** The border is `paneBusy`, which is working only, and the transcript in front of you already shows the question card or the failure banner. The build must not add a line there; a test pins it.
- How: `PaneChrome` passes the strip whether the pane is collapsed. The selected pill shows its line only then. The border is hidden for a collapsed pane in CSS: `.tile-minimized .pane-progress-bar-slot { display: none }`. That also stops the animation, so a collapsed pane costs nothing for it. The static selection ring stays.
- Because a collapsed pane's header shows **all** its pills (verified: the full `PaneHeaderTabStrip`, every stack member), each working tab in it gets its own line. One collapsed pane holding three working agents shows three moving lines.

Not chosen: moving the expanded pane's border to the line too. It would remove the one indicator that frames the whole pane and leave the pill's 2px as the only signal on a pane you are reading; and the border, which the operator did not ask to change, is the busy-ring work of #4934.

### 3.5 Motion (decision D5)

The line is category **`status`** in the motion policy, like the busy ring. It is gated in the stylesheet with `mixins.when-motion-reduced(status)`, never with a raw `prefers-reduced-motion` query (CI's `check-motion-gates.mjs` rejects one). Today the policy reduces nothing in either tier (`MOTION_POLICY = { full: [], reduced: [] }`), so the line sweeps whatever the OS says, which is what the operator wants; the operator keeps OS animations off.

The reduced treatment is declared and dormant, as the ring's is: the bar stops and parks, still, at the centre of the track (a 40% segment, not a full line), so a working pill is still different in shape from waiting (solid), limited (dotted) and signed out (dashed). A later change to the policy table turns it on for the pill, the ring and every other status animation together.

**This is motion, not a flash.** WCAG 2.3.1 limits flashes: more than three in any second, over a large enough area, with a large luminance change. The line is 2px high, never alternates between light and dark, and moves slowly (one pass per 1.6 s, soft-edged), so it is a different thing from the drop-target blink (see [SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md](SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md) §3). Many lines moving at once are still not a flash: the area is the sum of strips a few px high. WCAG 2.2.2 (pause, stop, hide) lets motion that is essential to an activity stand without a control; a working indicator is that, and the policy table is the off-switch if the operator ever wants one.

### 3.6 Colour (decision D3)

**Recommendation: the pill's own colour.** The pane tabs already carry an identity colour (`--pane-tab-underline`, set inline from `getColor`); the active underline and the activity flash use it, and with several working agents in one strip it is what tells the lines apart. An uncoloured pill falls back to `--accent-color`.

Not chosen: the Swarm "working" colour. It is `--error-color` (red); a red line on every working tab reads as an alarm, and the Swarm uses it to stand out in a list, not to sit on a tab.

Readability: the bar is the pill colour at full strength over the pill's own background; the track is 25%. The build checks the bar against the pill in the light and dark themes at 3:1 (WCAG 1.4.11) and, where a colour falls under it, lifts the bar the way the flash does (`oklch(from <colour> min(0.85, calc(l + 0.06)) …)`); the track need not meet it, being decoration beside the bar.

How it sits with its neighbours:

- **Activity flash:** the flash fills the pill's whole background for 300 ms or less; the line stays on top of it. They don't interfere, and a tool call during work is exactly when both show.
- **Tab-drag blink:** the blink is on the pane header's drop zone, not the pill; the line is unaffected. A pill being dragged leaves a drag image taken once, so its line is a still frame there.
- **Multi-select:** the picked pill's outline and tint are inside the pill; the line is above them and 2px.
- **Active underline:** at the bottom edge; the line is at the top. No overlap.

### 3.7 Cost (many agents working at once)

- The sweep is `transform: translateX()` on a 2px layer: composited, no layout, no paint after the first. `will-change: transform` is set only while working.
- One pill = one subscription (§3.3), and a re-render of the pill only when its progress value changes, not on every `controllerstatus` tick (the store keeps the derived value, not the payload).
- **Hidden:** a pill in a window tab that isn't shown (`usePaneTabVisibility` = `windowHidden`, and `dormant` for a pane whose tab isn't displayed) renders no sweep: the class that animates is not applied, and the subscription stays (it is cheap and keeps the value current for when the tab is shown). A window the OS has hidden or minimised is throttled by Chromium for compositor animations; the build adds nothing for it, and the live check in §8 reads the frame cost.
- The border for a collapsed pane is `display: none`, so collapsing a busy pane removes an animation.

## 4. Edge cases

- **A pill scrolled out of view in a long strip.** The line goes with it. This spec does not add an edge marker; it is a follow-up (D7), since the strip's blur edges and "reveal selected" already exist and a working-aware marker needs its own design.
- **A tab that is not an agent** (terminal, browser, Swarm): `paneTabProgress` is `null`, no line.
- **The agent finishes.** The sweep stops and the track fades over 200 ms (the ring's fade), so the end is not a hard cut.
- **A turn starts the instant a pill is selected, or the selected tab changes.** The selected pill (expanded pane) drops its line and the border takes over; a pill leaving the selection gets its line. Both are one frame.
- **A pane that is expanded while a collapsed one's lines were moving.** Nothing carries over: each pane decides from its own `collapsed` flag.
- **A magnified pane or a floating pane:** the same `PaneChrome`, so the same rule: an expanded pane's selected tab has the border.
- **A forked pane or a remote swatch:** the pill's colour is whatever `getColor` returns; the rules are unchanged.
- **Other windows:** each window draws its own pills from its own subscriptions. A pill is only ever drawn in the window that holds its pane, so "in other windows" is that window's own work, and a torn-off tab gets the same line.
- **Remote agents** (on other computers) have no pane tab in this window; the Swarm chip covers them.
- **The window tab (workspace tab bar)** still has no working indicator. The operator asked about the pane tab pill; a window-tab marker is a follow-up if wanted (D7).
- **RTL / zoom:** the line follows the pill's box (`left: 0; right: 0`), and `top` uses the ring width token, which scales with the strip's `zoom`.

## 5. Tests

Unit and component tests (jsdom), each written to fail without its change:

1. `pane-tab-progress.test.ts`: the table that pins the store to srv's `decide()` rows (working, waiting, limited with and without a reset, signed out, a turn beating a failure, idle, a shell block); the store opens one subscription per mounted block and closes it; an unchanged value does not notify.
2. `PaneTabStrip.progress.test.tsx`: a pill with `working` renders `.pane-tab-progress--working` and `aria-busy`; `waiting`, `limited`, `signed_out` render their classes and no sweep class; `null` renders nothing; the selected pill of an expanded pane renders none, and of a collapsed pane renders one.
3. `PaneChrome.test.tsx`: a collapsed pane passes `collapsed` and its selected pill shows the line; an expanded pane's does not; each of three working tabs in a collapsed pane has its own.
4. A stylesheet test (like `drop-feedback.test.ts`): the sweep animates only `transform` (no `left`, `width`, `top`), its duration and the track alpha are pinned, the reduced treatment is inside `when-motion-reduced(status)`, and `.tile-minimized .pane-progress-bar-slot` is `display: none`.
5. `check-motion-gates` keeps passing (no raw query).
6. The shared decide() fixture (`agent-state-decide.json`) read by `agent_state.rs`'s test and by `pane-tab-progress.test.ts`; the limited-to-idle row at the reset; a fake-timer store test that the line goes at `resetsAtMs` with no event; the accessible description's text for each state.

**Live (my own `task dev`, over CDP):** two windows; one collapsed pane with three agents (fake CLI with a long turn), one expanded; the selected agent working (border, no line), the others (lines); a collapsed pane's border gone; the top-edge geometry of §3.1; the lines in light and dark themes; frame cost with eight working lines.

## 6. Files the build touches

- `frontend/app/store/pane-tab-progress.ts` (new) and its test; `test/fixtures/agent-state-decide.json` (new); a test in `crates/srv/src/backend/agent_state.rs` that reads it.
- `frontend/app/element/PaneTabStrip.tsx`, `PaneTabStrip.scss`: the child element, the classes, `aria-busy`; a `getProgress` callback prop and a `collapsed` prop, passed through `PaneHeaderTabStrip.tsx`.
- `frontend/app/element/PaneChrome.tsx`: passes `getProgress` (from the store), `collapsed`, and the tooltip line.
- `frontend/app/element/PaneChrome.scss` (or `tilelayout.scss`): the collapsed pane hides the border slot.
- `frontend/app/_keyframes.scss`: `am-sweep` (the compaction bar's keyframes move here; the compaction bar uses the shared one).
- `frontend/app/view/agent/styles/_control-bar.scss`: use the shared keyframes.
- Tests above.
- Docs: the busy-ring spec gets a note that a collapsed pane no longer shows the ring; the activity-flash spec's §7.1 points here; the docs site's agent-pane page gets a paragraph (`agentmux-docs`, in the same change set as the build).
- A `minor` changeset.

## 7. Not in this spec

- A window-tab (workspace tab bar) indicator.
- An edge marker for a working pill scrolled out of view.
- A real percentage: there is none to show, apart from compaction's own estimate, which stays on its bar.
- Held reasons (launching, compacting, reconnecting) on background pills.
- Sound. The activity flash and its sound are unchanged.

## 8. Risks

- **The ring's top edge overlay.** If the line at `top: var(--pane-ring-width)` still clips under the ring in a focused pane, it moves down by the ring's width plus a pixel; found by the first dev build.
- **The collapsed border claim** is read from CSS; if the border is not in fact shown for a collapsed pane today, the `display: none` rule is a no-op and the spec's second ask is already true.
- **Many subscriptions.** One per visible pill is the same as the Swarm's per-block subscriptions and the agent views'; the store shares one listener per block between consumers.

## 9. Decisions for the operator

Each has a recommendation; the rest of the spec assumes the recommendations.

| # | Decision | Recommendation |
|---|---|---|
| **D1** | Which states get a line? | Working (moving bar), waiting (solid, accent), limited (dotted, warning), signed out (dashed, warning), as AgentX asked, each with its own shape. Nothing for idle. |
| **D2** | The border on an expanded pane's selected tab | **Keep it; the line on every other pill; no line on the selected pill of an expanded pane.** On a collapsed pane: lines only, selected pill included, border hidden. |
| **D3** | The line's colour | The pill's own colour (`--pane-tab-underline`, else accent); waiting accent; limited and signed out warning. Not the Swarm's red. |
| **D4** | Where the state comes from | The frontend derives it from `controllerstatus` and block meta by the rule srv uses, pinned by a table test. No srv change. (Alternative: add `agent_state` to the controller status event so the frontend reads srv's own answer; more exact, needs an srv and bindings change and a republish when a question is asked.) |
| **D5** | Motion | `status` category through the motion policy; moving in both tiers today; the reduced treatment (the bar parked still at the centre) declared and dormant. |
| **D6** | Speed and size | 2px, 40% bar, 1.6 s a pass. Tune on the dev build. |
| **D7** | Window-tab marker and an offscreen-pill marker | Not now; each is a small follow-up if you want it. |

Agent: Camper@narko
