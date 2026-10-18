# Report: a "locate" flash on the pane you just selected from Swarm

**Status:** implemented as proposed, with the choices in section 7 (the operator said to use best practice)
**Date:** 2026-10-18
**Requested by:** repo owner: "when clicking on an agent in the swarm pane, it selects the agent's pane for typing, but
sometimes it can be hard to tell who was selected. can you research best practice sort of flash with a slow but quick
recession that quickly gives the user indication of the pane, sometimes different windows or tabs switch, so the flash
needs to happen after the pane becomes available on the screen."
**Author:** AgentY
**Code read at:** `origin/main` e94f3ac0b.

---

## 0. Summary and recommendation

1. **Do it.** A one-shot "locate" flash on the target pane is a well-worn pattern (the "yellow fade" / "yellow flash").
   It answers "what just happened, and where?" after an action that moves your attention somewhere you were not looking.
2. **Hook it into `revealBlockLocally`, not into Swarm.** Swarm, Tower, the token popover and the agent picker all call
   `revealBlock`, and a cross-window reveal arrives as `block:reveal` and ends in the same function. One emit there
   covers every path, including the other-window one. Swarm's click handler stays as it is.
3. **The flash waits for the pane to be on screen.** A readiness gate (section 4) holds it until the pane's window tab is
   displayed, the pane's tab is the active one, the window is visible, and the pane has a stable, non-empty size. If the
   gate is not met within a timeout, there is no flash: a late flash on the wrong thing is worse than none.
4. **The look (section 3):** an instant rise, a short hold, then an ease-out fade, about 0.8 s in total. Three layers, all
   in the pane's own colour: a faint wash over the pane, a thicker ring that settles into the normal ring, and the pane
   tab pill (the agent's name) lighting up using the existing activity-flash overlay. Opacity only, so it runs on the
   compositor.
5. **One flash per reveal, at most three per second, newest wins.** That keeps it inside WCAG 2.3.1.
6. **Motion policy:** category `feedback`. Today the policy reduces nothing in either tier, and a fade is the substitute
   reduced-motion guidance recommends anyway, so no special reduced path is needed. Wire it through
   `shouldReduceMotion("feedback")` so a future policy change applies.

Open for you (section 7): how loud (three presets), and whether the flash fires on every reveal path or only from Swarm.

---

## 1. What happens today (verified in code)

| Piece | Where | Behaviour |
|---|---|---|
| Swarm row click | `frontend/app/view/swarm/swarm-view.tsx:437-440` | `stopPropagation()` then `void revealBlock(node.blockId)`. `mousedown` is cancelled so the Swarm pane never flashes selected first. |
| Reveal in this window | `frontend/app/util/reveal-block.ts` `revealBlockLocally` | Finds the window tab holding the block, `setActiveTab`, **polls up to 10 x 50 ms** for the layout to contain the block, `showBlockInPane` (restore if minimized, un-magnify another pane, switch the pane's tab), `model.focusNode`, then `giveBlockFocus` (caret). |
| Reveal in another window | `revealBlock` -> `revealInOtherWindow` -> `BlockRevealCommand`; the named window's renderer gets `block:reveal` (`reveal-block-events.ts`) and runs `revealBlockLocally`; **the calling window raises it** with `raiseWindowById` -> `focusWindow`. | The target window may still be hidden or behind others while it is revealing. |
| Selection cue | `frontend/app/element/PaneChrome.scss:47-95` | A `::after` overlay with a 2 px ring (`--pane-ring-width`) in `--pane-ring-color`, shown by `.pane-stack-focused`. Nothing animates when focus arrives. |
| Existing flash machinery | `frontend/app/notification/activity-flash.ts`; used by `PaneTabStrip.tsx:691` (the pill) and `tab.tsx:260` (window tab) | A per-block pub/sub bus plus `flashElement`, which plays an **opacity-only** Web Animations keyframe set on the element's `::before` overlay. The pill and window tab already carry that overlay and its stylesheet. Tail is a fixed 280 ms (`FLASH_TAIL_MS`). |
| Other landing flash | `frontend/app/drop-feedback.scss` `tab-bounce` 300 ms; `tabbar.scss` strobe | For drag and drop; it strobes and is unsuitable here (section 5). |
| Motion tokens | `frontend/app/motion/tokens.ts`, `theme.scss:427-458` | Durations top out at 280 ms (`--duration-slow`); `--ease-decelerate` and `--ease-emphasized` exist. A 0.8 s fade needs a new token or a local constant. |
| Motion policy | `frontend/app/motion/motion.ts` | Categories `status`, `feedback`, `transition`, `decorative`. As of 2026-10-15 every category runs in both tiers. |

So the selection is signalled by a thin static ring appearing, which is easy to miss when several panes look alike
(agent panes share a layout), and when a tab or window switch changes the whole screen at once.

---

## 2. What the research says

Findings, with what is sourced and what is my inference marked.

**The pattern.** 37signals' "yellow fade technique" flashes a recently-changed area and fades it back, so people can
confirm what changed after a page update or a jump; the effect is unobtrusive and lasts "a second or two". CSS-Tricks
calls the same thing the "yellow flash" for the target of an in-page link: a *temporary* highlight draws the eye, a
permanent one does not. Both warn that it loses its value if many things flash at once.
Sources: [Signal vs. Noise](https://signalvnoise.com/archives/000558),
[CSS-Tricks, The Yellow Flash](https://css-tricks.com/books/greatest-css-tricks/the-yellow-flash/),
[a one-second CodePen version](https://codepen.io/Mestika/pen/KVGwKb).

**Timing.** Neither Material nor Apple HIG specifies a "highlight pulse". The nearest guidance: Material 3 short
durations are 50-200 ms and medium 250-400 ms, with decelerate-style easing (`cubic-bezier(0.2, 0, 0, 1)`) for things
entering; GitLab's design system uses 500 ms for action feedback and 600 ms for position changes. A third-party Apple
summary says to keep animations under 0.5 s (I could not confirm that on Apple's own page).
Sources: [Material 3 easing and duration](https://m3.material.io/styles/motion/easing-and-duration/tokens-specs),
[GitLab motion](https://design.gitlab.com/product-foundations/motion).
*Inference:* a highlight that must be noticed in peripheral vision needs longer than a state change, because the eye has
to arrive before it ends. That is why the yellow-fade family runs 1-2 s, and why I propose about 0.8 s: the pane is a
large target and the user is usually already looking at the click point.

**Shape of the curve.** The jump-to-message recipe holds the full colour for the first 20-30 % and then fades out, so
the highlight is still visible when a scroll settles (a sketch in the search result, not a published Slack or Discord
value; neither app documents its timing). *Inference:* "slow but quick recession" is best read as an instant rise, a
brief hold and a soft ease-out tail, never a symmetric pulse.

**Scroll or switch first, then flash.** GitLab and the same jump-to-message recipe both start the highlight after the
movement settles. This is the point the owner raised, and section 4 is built around it.

**Accessibility.**
- WCAG 2.3.1 (Level A): no more than three flashes in any one second, unless below the general and red flash
  thresholds; the area test is roughly a 341 x 256 px region (about 25 % of a 10 degree field). Our own comment in
  `activity-flash.ts` relies on that small-area exemption for a pill. **A whole pane is larger than the exemption**, so
  a pane-sized flash must be a single, slow, low-contrast event. A slow fade of low alpha is safe; strobing opacity is
  the failure case.
  Sources: [WCAG 2.3.1 summary](https://wcag.dock.codes/documentation/wcag231/),
  [BOIA, accessible web animations](https://www.boia.org/blog/making-your-web-animations-accessible-5-tips).
- WCAG 2.3.3 (AAA) concerns *motion* animation; opacity and colour fades are out of its scope, and replacing motion
  with a fade is the standard fallback.
  Source: [WCAG 2.3.3](https://accessibility.build/wcag/2-3-3).
- Discord's Reduced Motion setting removes the slide on jump-to-message and goes straight to the message
  ([Discord support](https://support.discord.com/hc/en-us/articles/360040613412-Reduced-Motion-Setting)). Our policy
  already runs everything in both tiers, so this is a note for later, not a change.

**Performance.** `box-shadow` and `outline` are paint properties; animating them repaints every frame, with the Web
Animations API or with CSS alike. `opacity` and `transform` can stay on the compositor even while the main thread is
busy. The usual fix is a static shadow or ring on a pseudo-element and animating only its opacity.
Source: [SitePoint, animating box-shadow](https://www.sitepoint.com/css-box-shadow-animation-performance/),
[Tobias Ahlin](https://tobiasahlin.com/blog/how-to-animate-box-shadow/). This matches the choice already made in
`activity-flash.ts` (opacity only) and the reason that file cites for avoiding a forced reflow per trigger.

**Tiling terminals and window managers.** I found no mainstream precedent for an animated focus flash in tmux, i3 or
yabai; they use a static active-pane border, and tmux's `display-panes` overlays pane numbers for a few seconds.
(Search came back empty on animated focus in Hyprland and yabai; I did not verify those from memory.) So this is a place
AgentMux can do better than the terminal-multiplexer baseline, not a convention to copy.

---

## 3. Proposed look

Interpretation of "slow but quick recession": appears at once, fades gently. Three layers, all driven by one animation
timeline and all in the **target pane's own colour** (`--pane-ring-color`, the same colour Swarm already uses for the
selected row), so the colour itself says whose pane it is.

| Layer | What | Peak | Why |
|---|---|---|---|
| Wash | An `inset: 0` overlay filling the pane in the pane colour | alpha 0.16 dark theme, 0.12 light | The big, peripheral cue; fades to nothing. |
| Ring | The 2 px ring thickens to about 4 px and settles back | 0.9 -> 0 on a second ring layer | Ties the flash to the permanent selection cue so the eye reads "this is the selected one". |
| Pill | The pane tab pill (agent name) lights up through `flashElement` | one strike, intensity 1 | The name is the answer to "who". Reuses the existing overlay. |

Envelope (all layers): rise 0-60 ms, hold to 140 ms, ease-out decay to 0 by about 900 ms.

| Preset | Total | Wash peak | When to pick it |
|---|---|---|---|
| Quick | 500 ms | 0.10 | Minimal; closest to GitLab's 500 ms. |
| **Standard (proposed)** | 900 ms | 0.16 | Visible in peripheral vision, gone before it nags. |
| Slow | 1400 ms | 0.20 | Closest to the classic yellow fade. |

The numbers are a starting point to tune by eye in a dev build; they are not measured. `activity-flash.ts` keeps its
fixed 280 ms tail, so the pill needs a tail argument (or the pane flash uses its own envelope on the pill's overlay).

Implementation shape: the wash and extra ring are one absolutely positioned, `pointer-events: none` element that exists
only while the flash runs (mounted by `PaneChrome` on demand, removed on `finish`), animated with `el.animate()` on
`opacity` only. No cost at rest, no `will-change` left behind, no reflow to re-trigger.

---

## 4. "After the pane is on screen": the readiness gate

The flash must not start at the moment `revealBlock` is called. That moment can be well before anything is visible:
`revealBlockLocally` itself polls up to 500 ms for a not-yet-built tab layout, and a cross-window reveal runs in a window
that may still be hidden. Proposed gate, evaluated by the pane that owns the block (`PaneChrome`), which already has
the signals:

1. **The pane exists and shows the block.** `activeBlockId() === blockId` (a background tab of a multi-tab pane becomes
   active only after `showBlockInPane`).
2. **Its window tab is displayed and the tab is not dormant.** `usePaneTabVisibility(blockId) === "active"`
   (`block/pane-tab-visibility.ts`: not `windowHidden`, not `dormant`).
3. **The window is visible.** `document.visibilityState === "visible"`. For a cross-window reveal this is what waits for
   `focusWindow` to take effect. If not visible, listen for `visibilitychange`.
4. **It has a real, stable size.** A `ResizeObserver` reports a non-zero rect that is unchanged across two animation
   frames. This also covers a pane that was minimized, restored or un-magnified a moment ago.
5. **Then start on the next frame.** Two `requestAnimationFrame` calls after the last condition becomes true, so the first
   composited frame with the pane in place exists before the animation clock starts. Set the animation's start time
   explicitly, as `activity-flash.ts` does, so it does not begin up to a frame late.
6. **Give up quietly.** If the gate is not met within about 1.5 s (3 s when another window is involved), drop the flash.
7. **Cancel when it no longer applies.** If the focused pane changes to something else before the flash starts or while
   it runs, cancel it: it must never mark a pane the user has already left.

Why visibility matters: a document that is hidden or fully occluded does not produce frames, so a short animation can
run its whole length unseen and the user sees only the end. *Inference from how the animation timeline works; to confirm
in the dev build with a hidden second window.* This is the one place a plain CSS `animation` on mount would fail the
owner's requirement.

Cases the gate must handle, and how each is covered:

| Case | What the gate waits for |
|---|---|
| Same tab, pane already visible | Passes at once; flash starts next frame. |
| Background pane tab in the same pane | Condition 1 (active block switches). |
| Another window tab in this window | Condition 2, plus the layout build `revealBlockLocally` already waits for. |
| Pane was minimized or another pane was magnified | Condition 4 (size and stability after restore or un-magnify). |
| Another window, hidden or behind | Condition 3, with the longer timeout. |
| Agent pane showing a loading view | The pane chrome is up when the view body is not; the flash is on the chrome, so it does not wait for content. |

---

## 5. Rejected alternatives

| Option | Why not |
|---|---|
| Reuse the Window Tab invert strobe (`tab-drop-invert-strobe`) | Hard inversions, repeated, over a whole pane exceeds the small-area exemption. It exists for drag hover on a small target and stays there. |
| Scale "pop" or bounce (`tab-bounce`) | Moves the pane's pixels, so text under the cursor shifts; it is motion, not just a cue. |
| Infinite or repeating pulse | Needs a stop condition and risks the flash-rate limit; one decaying flash carries the same information. |
| Dimming every other pane (spotlight) | Strong, but changes the whole screen on every click in Swarm, which is exactly the noisy case. Could be a future "locate" command, not this. |
| Animating `box-shadow` or `outline` | Repaints every frame; the layered-opacity version looks the same. |
| Flash only the Swarm row | The row already shows a selected border; the missing cue is on the pane. |
| A stronger permanent ring | Worth considering separately, but it does not help in the moment a window or tab switches. |

---

## 6. Where it plugs in

1. **New small module** `frontend/app/util/pane-locate.ts` (or beside `activity-flash.ts`): a per-block pub/sub like
   `emitActivityFlash`, `emitPaneLocate(blockId, { crossWindow })`, store-free, with a rate limiter (at most one flash
   in flight, starts at least 340 ms apart so the limit is three a second, newest wins).
2. **`revealBlockLocally`** emits after `giveBlockFocus`, on the success path only. That covers `revealBlock` from Swarm,
   Tower (`tower-agents.tsx:242`), the token popover (`TokenBreakdownPopover.tsx:112`), the agent picker
   (`AgentPicker.tsx:620`) and `block:reveal` from another window.
3. **`PaneChrome.tsx`** subscribes for its pane's blocks, runs the gate, mounts the overlay, plays the animation, and
   calls `flashElement` on its pill. Styles go in `PaneChrome.scss` next to the ring rules they mirror.
4. **Motion:** a `feedback` category check via `shouldReduceMotion("feedback")`; durations as named constants in
   `motion/tokens.ts` (they are longer than the current top token) with a matching token in `theme.scss` so the
   existing token test keeps them in step.
5. **Not touched:** Swarm's click handler, the caret logic, `showBlockInPane`. The flash is purely additive and never
   changes what is focused.

Tests, in the style of `activity-flash.test.ts`: the gate (each condition holding the flash back, then releasing it),
cancel on focus change, the rate limiter, one animation per reveal, and no flash when `revealBlockLocally` returns
false. The look itself needs a human in a `task dev` build, including a second window and a hidden window; a screenshot
tool cannot show an 0.9 s fade, so check it with a short screen recording or `getAnimations()` in DevTools.

---

## 7. Decisions

The operator answered "use best practice, proceed", so the proposals were taken:

1. **Loudness:** Standard (0.9 s, wash 16 % dark / 12 % light).
2. **Scope:** every reveal path (Swarm, Tower, token popover, picker, cross-window).
3. **Pane already selected and visible:** still flashes, because Swarm confirms the click.
4. **Still open, separately:** whether to strengthen the permanent selection ring.

What was built differs from section 6 in two small ways: the gate reads what the browser paints
(`checkVisibility`, document visibility, the pane's box unchanged for two frames) instead of the individual tab and
dormancy signals, which also covers the window tab's own settle step; and the pane owns the request/play pair, so a
reveal elsewhere ends the running flash. The code is `frontend/app/util/pane-locate.ts` and the hooks in
`PaneChrome.tsx`, `PaneTabStrip.tsx` and `reveal-block.ts`.

## 8. Sources

- [Web Interface Design Tip: The Yellow Fade Technique, Signal vs. Noise](https://signalvnoise.com/archives/000558)
- [The Yellow Flash, CSS-Tricks](https://css-tricks.com/books/greatest-css-tricks/the-yellow-flash/)
- [Yellow Fade Technique, CodePen](https://codepen.io/Mestika/pen/KVGwKb)
- [Material 3: easing and duration tokens](https://m3.material.io/styles/motion/easing-and-duration/tokens-specs)
- [GitLab Pajamas: animation fundamentals](https://design.gitlab.com/product-foundations/motion)
- [WCAG 2.3.1: Three Flashes or Below Threshold](https://wcag.dock.codes/documentation/wcag231/)
- [WCAG 2.3.3: Animation from Interactions](https://accessibility.build/wcag/2-3-3)
- [BOIA: Making your web animations accessible](https://www.boia.org/blog/making-your-web-animations-accessible-5-tips)
- [Discord: Reduced Motion setting](https://support.discord.com/hc/en-us/articles/360040613412-Reduced-Motion-Setting)
- [SitePoint: animating box-shadow and performance](https://www.sitepoint.com/css-box-shadow-animation-performance/)
- [Tobias Ahlin: animating box-shadow](https://tobiasahlin.com/blog/how-to-animate-box-shadow/)

Agent: AgentY@narko
