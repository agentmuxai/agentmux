# Window-tab switches: why they still look jittery next to pane tabs, and how to make them one frame

**Date:** 2026-09-30
**Status:** analysis. Findings are in §2–§3 and recommendations in §5. Recommendations 1 (explicit transition lists plus a CI gate, without the reveal backstop) and 2 (the optimistic warm swap) are implemented in the PR that adds this doc, with the results in §7. 3–5 are not yet.
**Author:** korp
**Trigger:** Repo owner, 2026-09-30: *"The pane tabs switching are instant and fast, but window tabs are still jittery and slow. is there a img fix for that too? can we make it faster?"*
**Related:** `ANALYSIS_WINDOW_TAB_SWITCH_SMOOTHNESS_2026_09_24.md` (the 09-24 analysis: keep inactive tabs laid out, #3686/#3687; its items 3 and 5 are still open), `SPEC_TEAROFF_PAINT_LATENCY_2026_09_30.md` (the tear-off snapshot work this question refers to), `SPEC_TAB_CONTENT_REVEAL_GATE.md`, `ANALYSIS_CHROME_PARITY_TAB_SWITCH_LATENCY_2026_09_15.md`.

Code citations are against `main` @ `6640ddd9e`.

---

## 1. Summary

The 09-24 work fixed the expensive part of a window-tab switch. Hidden window tabs now stay laid out, so a switch to a tab that has already been shown has no layout to catch up on, no reveal gate and no fade. That work is in place: over 12 traced switches, no frame took longer than 20 ms.

What is left is not slowness on the main thread. **Each switch now paints in three separate frames where a pane-tab switch paints in one:**

1. **About 22 ms after the click:** the pill turns active. It is optimistic (#2993).
2. **About 46 ms:** the content swaps. It waits for the `SetActiveTab` round trip.
3. **About 65 ms:** part of the new tab pops in one frame late. On the traced tabs, these are the agent picker's cards and recent-session rows.

Step 3 is a bug. **Any element with `transition: all` animates the inherited `visibility` flip that hides and shows window tabs.** Going from hidden to visible, the first frame of that transition is still hidden, so the element appears a frame after the rest of the tab. Going from visible to hidden, it stays `visible` for the whole transition (150–300 ms), but the leaving tab's `opacity: 0` keeps that from showing. Removing `visibility` from those transitions made step 3 disappear on all 12 switches (§3).

Step 2 is the round trip. The pill already switches without waiting for it, and the content could too, now that a warm tab swaps in one frame.

**Is there an image fix, as for tear-off?** Not for warm switches. The destination tab is live, laid out and one frame away. A snapshot would be slower to put on screen than the real tab, and it could be stale. A snapshot only fits the **first** visit to each tab in a session, which still takes the gated path (§4).

## 2. What a switch costs now (measured)

**Setup:**
- dev build of `main` plus #4095/#4097, one window with 4 window tabs;
- each tab holds an agent picker, a sysinfo chart and swarm panes;
- every tab had already been shown this session (the warm path);
- 12 real clicks on the tab pills through DevTools `Input.dispatchMouseEvent`, 1.5 s apart.

**Instruments:**
- an in-page probe recording the pointerdown time, the pill `active` class change, the tab containers' style flip and every `requestAnimationFrame`;
- a DevTools trace with `disabled-by-default-devtools.screenshot`, giving one screenshot per presented frame. Consecutive screenshots were diffed, per region, to see what changed in each frame.

| Per switch, n = 12 | Measured |
|---|---|
| Pill `active` in the DOM | 3.1–4.3 ms after pointerdown |
| First frame showing the active pill | 20–24 ms |
| `SetActiveTab` (host log) | median 28 ms, 26–41 ms |
| Tab containers' visibility flip in the DOM | 28.5–31.7 ms |
| First frame showing the new content | 43–50 ms |
| **Another frame changing 5.3% of the content area** | **62–72 ms, 12 of 12 switches** |
| Frames longer than 20 ms in the 600 ms after the click | 0 of 12 |
| Reveal gate | ungated, 0 ms, on all 12 (`[perf] tab-reveal … source=ungated`) |

The screenshot strip for one switch shows what the third frame is. At 44 ms the destination tab is in place, but the agent picker's cards and "Hot agents" row are empty boxes. At 64 ms they fill in. On every switch the frame-3 change was confined to the same box, the picker pane.

For comparison, a pane-tab switch is one local signal change and paints in the next frame (09-24 analysis §3).

## 3. Root cause of the late frame: `transition: all` animates `visibility`

Window tabs hide inactive tabs with `visibility: hidden` on the tab container (`frontend/app/workspace/window-tab-visibility.ts`, applied in `workspace.tsx`). Every descendant inherits it, so on a switch each descendant's computed `visibility` changes. `visibility` is animatable, so any element whose `transition-property` includes it starts a CSS transition on that change. `transition: all` includes it.

CSS interpolates `visibility` as a discrete step. When one endpoint is `visible`, every progress value from 0 to 1 exclusive maps to `visible`, and the endpoints map to themselves. So:
- **hidden → visible (the destination):** the first frame samples progress 0, which is `hidden`. The element appears one frame late. That is step 3.
- **visible → hidden (the source):** the element stays `visible` until the transition ends, which takes 150 ms for the picker and 300 ms for `.wave-button`. It stays invisible, though: a hidden, laid-out tab also gets `opacity: 0` (`tabContainerVisibility`, `SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md` §1), which nothing escapes. So the visible symptom is the late frame on show.

**Confirmed live:**
- `document.getAnimations()` one frame after each switch held **56 running CSS transitions, all with `transitionProperty === "visibility"`**: 40 on `.agent-recent-sessions-entry` and 16 on `.agent-card`. There were none of any other kind.
- Injecting one rule limiting those classes' `transition-property` to colours, borders, shadow, opacity, transform and filter, then re-running the 12 switches: **the 62–72 ms frame was gone on all 12.** The content now lands whole in the swap frame, 47–58 ms.

The rules that do this today, 12 in all:

| File | Selector / line | Duration |
|---|---|---|
| `app/element/button.scss:26` | `.wave-button`, **every Button** | 0.3 s |
| `app/element/modal.scss:181` | modal | `--motion-fast` |
| `app/view/agent/styles/_composer-strip.scss:260` | composer strip, in **every agent pane** | `--motion-fast` |
| `app/view/agent/styles/_connection-status.scss:46` | connection status | 0.15 s |
| `app/view/agent/styles/_header-controls.scss:83` | agent header controls, in **every agent pane** | 0.15 s |
| `app/view/agent/styles/_picker.scss:284, 599` | `.agent-card`, … | 0.15 s, 0.1 s |
| `app/view/agent/styles/_recent-sessions.scss:242` | `.agent-recent-sessions-entry` | 0.15 s |
| `app/view/agent/styles/_setup-wizard.scss:261, 336` | setup wizard | 0.15 s |
| `app/view/native-memory/native-memory-manager.scss:90, 167` | memory manager | 0.15 s |

The traced tabs only showed the picker. A tab of working agent panes carries the composer strip, header controls and buttons, so it will pop in the same way, in more places.

The same mechanism applies to **pane tabs**, which also hide with `visibility: hidden` (`pane-leaf-chrome.tsx`). It doesn't show there yet only because the members switched so far have no `transition: all` descendants in view.

## 4. The two remaining paths

### 4.1 Warm switch (tab already shown this session): the common case

- The pill is optimistic (`tabbar.tsx` `pendingSelectedTabId`, `active-tab-display.ts`). It paints in the first frame.
- The content reads the backend-authoritative `activeTabId` (`workspace.tsx` `displayTabId`). It flips only once `WorkspaceService.SetActiveTab` has round-tripped and the Workspace push has landed, about 28 ms, so it paints one to two frames after the pill.
- `active-tab-display.ts` explains why the split was made: the content reveal *used to* cost 500–600 ms of layout (`display: none`), and the pill shouldn't wait behind that. With `window:keepinactivetabslaidout` on by default, a warm reveal costs one frame (§2), so that reason no longer applies.
- `refocusNode()` still runs two frames after the round trip (`tab-actions.ts`). That is the 09-24 analysis's open item 5.

### 4.2 Cold switch (first visit to a tab this session)

- `tabWasShown` is false, so the switch holds the reveal gate, waits for 80 ms without long tasks (`tab-reveal.ts`), then runs the 120 ms View Transition fade (`workspace.tsx`).
- In this instance's log a first visit revealed at `source=settle elapsed=82ms` after a 9 ms round trip. With the fade, that is about 200–230 ms before the tab is fully shown, against about 50 ms warm.
- After every launch, **every** tab is cold once. So the first pass through the tabs is the slow path, however well warm switches perform.

## 5. Recommendations, in order

Each is small and can ship on its own. Measure each with the §6 method; the bar is **new content and active pill in the same frame, and no later frame changing the content area** that the switch caused.

1. **Stop `transition: all` animating `visibility` (fixes step 3, both directions).** Two layers:
   - **Replace the 12 `transition: all` rules with explicit property lists.** This is the standard CSS guidance anyway: `all` also animates layout properties and anything added later. Most need `color, background-color, border-color, box-shadow, opacity` (plus `transform` or `filter` where used). Keep it from coming back with a CI grep gate (`scripts/check-no-transition-all.sh`). stylelint isn't run in CI and has hundreds of pre-existing findings, so a rule there wouldn't hold.
   - *(Not done: the gate below keeps new ones out, and the source side is already masked by `opacity: 0`.)* **A backstop for hidden containers:** give inactive window-tab containers and hidden pane-tab members `transition: none !important` for all descendants, from the moment the container is shown until one frame later. A newly shown tab then never samples a transition's first frame, whatever its descendants declare. This could be a `data-revealing` attribute set with the visibility flip and removed on the next `requestAnimationFrame`. The list edit alone is enough for today's code; the backstop protects against the next `transition: all`.
2. **Show a warm destination optimistically (fixes step 2).** Lift `pendingSelectedTabId` out of `tabbar.tsx` into a shared intent signal, and have `workspace.tsx`'s `displayTabId` follow it **when the destination is warm** (`keepInactiveTabsLaidOut() && tabWasShown(id)`). The pill and the content then change in the same frame, about 20 ms after the click instead of about 46 ms. That is what Chrome does: the strip and the content switch together. Details:
   - The backend stays authoritative. `driveTabSelection` already converges on the latest click. If the backend lands somewhere else (the tab was closed, or another window moved it), `displayTabId` follows `activeTabId` once the intent clears, exactly as the pill does today.
   - Cold destinations keep the current gated path.
   - Anything reading `activeTabId` (focus, `getLayoutModelForStaticTab`, keyboard routing) agrees again one round trip later, as today. The only change is that the visible content no longer waits for it.
   - This is the 09-24 analysis's item 3, now cheap because the warm reveal is one frame.
3. **Focus in the same task as the show** (the 09-24 analysis's item 5). Call `refocusNode()` when `displayTabId` flips to a warm tab, with `preventScroll`, not two frames after the round trip. After 2, typing straight after a click lands in the new tab.
4. **Make the first visit warm too, without a snapshot.** Every tab is already mounted and laid out while hidden. What it lacks on a first visit is a first paint, and the gate waits for that. Once the window is idle after launch (`requestIdleCallback`, one tab per idle slot), paint each hidden tab once: make it `visibility: visible` for a single frame, stacked *under* the active tab so nothing shows, then hide it again and `markTabShown`. Every later switch then takes the warm, one-frame path. Measure first:
   - raster memory and time for tabs with many panes;
   - whether dormant agent panes wake for that frame. If they do, gate the pre-paint on dormancy or skip agent-heavy tabs;
   - ship it behind a setting, as #3686 was, so it can be A/B'd live.
5. **Only if 4 can't be made cheap: a snapshot for the first visit.** Keep a per-tab image, taken with the tear-off path's `capture_window_viewport` crop (`tearoff-snapshot.ts`) whenever a tab is hidden, and show it for the gated first visit instead of the fade. Content can be stale, especially after a restart, so the live tab has to replace it the moment the gate lifts. This makes it a worse fit than 4 and not worth building before 4 is measured.

**Not recommended:**
- A snapshot for warm switches (see §1). The live tab is faster and never stale.
- Longer or more gates. A gate can't hide a transition that starts after it lifts, and step 3 happens after the swap frame.

## 6. How this was measured (repeatable)

1. Connect to the dev instance's DevTools port (`AGENTMUX_CDP_PORT`, default 9223 in dev) and pick the main window's page target.
2. Install the probe:
   - a capture-phase `pointerdown` listener on `.tab[data-tab-id]`;
   - a `MutationObserver` on the tab bar (`class`) and on the `view-transition-name: workspace-tab-content` region (`style`);
   - a `requestAnimationFrame` loop recording frame times.
3. Start `Tracing.start` with `devtools.timeline`, `disabled-by-default-devtools.timeline.frame` and `disabled-by-default-devtools.screenshot`.
4. Click the pills with `Input.dispatchMouseEvent` (move, press, release), 1.5 s apart, visiting only tabs already shown.
5. Per switch:
   - take the probe times;
   - decode the `Screenshot` events after the `pointerdown` `EventDispatch`;
   - diff consecutive screenshots per region (tab bar, content), with a pixel threshold of 24/255;
   - count `document.getAnimations()` by `transitionProperty` one frame after the flip.

A switch passes when the pill and the content change in the same screenshot and no later screenshot within 200 ms changes the content area, allowing for live content such as charts and streaming agents.

The scripts used here are about 150 lines of Node plus a small Pillow diff. They should land as `scripts/tab-switch-frames.mjs` with recommendation 1, so the before and after of each step is on record.

## 7. Implemented and measured (recommendations 1 and 2)

**What shipped:**
- The 12 rules list their properties, and `scripts/check-no-transition-all.sh` (CI) keeps `transition: all` out.
- `setActiveTab` publishes a switch intent before its RPC (`tab-actions.ts` `switchIntentTabId`). It is cleared when the committed `activeTabId` catches up, when the RPC fails, or when a newer switch replaces it; never merely because the RPC returned, since the Workspace push can land after the reply. `workspace.tsx` displays a warm destination from it (`resolveDisplayedTabId`).
- **One addition the first trace called for.** Swapping in the input task also moved the swap's own cost there. On a large tab that cost is mostly the inherited `visibility` / `pointer-events` flip restyling every element of *both* tabs. So the tab being left now takes `content-visibility: hidden` for two frames, under the displayed tab (`z-index: 1`), and only then its `visibility: hidden` / `pointer-events: none` (`tabContainerVisibility`'s `leaving`). `content-visibility` isn't inherited and also skips the tab's paint. Hiding it with `opacity: 0` alone was tried first: it halved the style work, but the leaving tab was still painted, which cancelled the gain.

**A/B on one dev instance, same 7 window tabs** (two heavy: 6,800 and 11,000 elements; five light: about 970 each). 12 warm switches per run; frame times from trace screenshots; runs taken back to back:

| Median, 12 switches | `main` | this change |
|---|---|---|
| Pill on screen | 21 ms | 58 ms, together with the content |
| **Content on screen** | **71 ms** (46–101) | **58 ms** (31–87) |
| Switches with a later content step | 5 / 12 | **0 / 12** |
| Switch into a light tab | 46–57 ms | 31–38 ms |

**Reading it:**
- The late frame is gone.
- The pill and the content change in one frame.
- The content arrives about 13 ms sooner on the median, and about 15–20 ms sooner into light tabs.
- **The trade:** on a heavy tab, the pill now waits for the content instead of showing 40–60 ms ahead of it. That is how Chrome and pane tabs behave. The pill no longer promises a tab that isn't there yet.

**What's left, and the next step.** A switch into a heavy tab still costs 70–90 ms. That is restyling every element of the incoming tab (its inherited `visibility` flips back) and painting it for the first time since it was hidden, and every `visibility`-based hide has to pay it.

The way past it is to give each window tab its own compositor layer (`will-change: opacity`) and hide it with `opacity: 0` alone. A switch then restyles and repaints nothing on the main thread: the compositor just draws a different layer, as Chrome swaps tab surfaces. Before doing that, measure and solve:
- GPU memory, roughly one window-sized layer per tab;
- raster of hidden layers while agents stream (they are dormant, so probably small);
- keyboard focus and hit-testing, since `opacity: 0` content is still focusable. `inert` would bring back the subtree restyle, so this needs its own design.

## 8. Two paths that still flashed (2026-10-01)

After §7 the repo owner reported one flash left in two places: closing a window tab, and the first switch to each tab after a window loads. Both were the reveal gate hiding a destination that had nothing left to settle. The same per-frame trace (4 closes, then 8 switches after a reload):

| | Before | After |
|---|---|---|
| Close the active tab | blank content area at ~40 ms, neighbor at ~150 ms (26% of the area each way), 4 / 4 | neighbor in one frame at ~31 ms, 0 / 4 blank |
| First switch to each tab after load | blank at ~90 ms, tab at ~155 ms, 3 / 3 cold tabs | one frame at 29–41 ms, 0 / 4 |

**Close.** `tabbar.tsx`'s close path always held the gate on the neighbor that `CloseTab` promotes. It never went through `setActiveTab`, so it missed #4107's warm path. `beginClosePromotion` (tab-actions.ts) now shows a warm neighbor from the switch intent in the click's frame. A neighbor that was never shown is still gated, and a failed close drops the intent.

**First switch after load.** No tab counted as shown until it had been displayed once, so each first visit was gated and cross-faded. Kept laid out, every tab a window loads with is mounted and laid out behind the displayed one, which is the warm state. These are marked shown once each tab's content has settled (§9). That replaced a first version that marked them at the window's first idle moment: Codex pointed out that idle doesn't mean a tab's layout and data have loaded. A tab arriving later is still gated on its first reveal.

## 9. A new tab showed its panes loading

After #4108 a new window tab was shown as soon as its panes *existed*, about 75 ms after Alt+T, but their first data was still arriving. The trace showed several states in sequence:
- **75 ms:** the agent pane's loading cover (the brain), an empty sysinfo chart, and swarm's "Loading…";
- **100–140 ms:** the chart and the swarm list filling in;
- **140–170 ms:** the picker fading in over the cover.

#4108 had removed the gate and the fade that used to blur this.

**What shipped:** a tab is shown only once its content has *settled*. That means its layout has loaded, and every leaf pane is mounted with no content hold outstanding (`pane-content-holds.ts`, `tab-content-settled.ts`).

**Where the holds come from:**
- A pane's loading cover holds its content until it is gone (`createPaneReadiness`'s `holdFor`, set by `block.tsx`, `usePaneReveal` and `AgentPicker`).
- A view whose first data arrives outside the cover takes its own hold: sysinfo until its history arrives, swarm until its first list.
- A cover that finishes while its tab is hidden goes straight to `live` (`hidden`): nobody would see that fade, and it would only delay the tab.

**Who waits:**
- `createTab` waits, hidden, until the new tab has settled (capped at 800 ms), then switches to it in one frame.
- The tabs a window loads with are marked shown as each one settles (`markLoadedTabsShownWhenSettled`), not merely when the window first goes idle.

**Measured** (Alt+T, 3 new tabs, trace screenshots): the pill appears at 15–29 ms. The tab then appears **complete in one frame at about 153 ms**: picker, chart and swarm already in place, with no cover, no empty chart and no fade.
