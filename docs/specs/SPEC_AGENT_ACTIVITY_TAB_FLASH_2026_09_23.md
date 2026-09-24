# Agent tool-call tones get a visual twin: a subtle flash on the tab that made the sound

**Status:** implemented — PR #3625; see §8 for
where the shipped code departs from §2–§4 as first written.
**Date:** 2026-09-23.
**Requested by:** repo owner (asafebgi).
**Author:** Agent2.
**Related:** `docs/specs/SPEC_AGENT_TOOL_CALL_TONES_2026_06_05.md` (the tones
this complements), `docs/specs/SPEC_OS_TASKBAR_AGENT_ACTIVITY_INDICATOR_2026_05_23.md`
(the same "which thing is busy?" question, one level up at the OS taskbar),
`docs/specs/SPEC_COLOR_THEME_TOKEN_HARDENING_2026_09_21.md` (color rules).

---

## Revision 3 (2026-09-24): the window tab is subtle and uncolored; the pill is less bright

Owner feedback on Revision 2. It replaces Revision 2's item 2 (color) for both
targets and the peak opacity in item 3. Everything else in Revision 2 stands.

1. **Window tab:** it never uses the pane's color. Its flash is a slight
   offset of the tab's own normal color: a `--main-text-color` tint at
   **0.12** alpha over whatever the tab already is, its own `tab:color`
   included. That is just above the 0.1 hover tint. The tint is theme-aware,
   so it lifts dark tabs and darkens light-theme tabs. The tab passes no base
   color. The pane's color belongs to its pill.
2. **Pane pill: less bright.** It is still the pane's own color
   (`--pane-tab-underline`, else accent), brightened less:
   `oklch(from <base> min(0.85, l + 0.06) c×1.4 h / 0.9)`, down from
   `min(0.92, l + 0.15) c×1.6`. Measured live: an amber `#f59e0b` pill peaks
   at `oklch(0.83 0.23 70)` (was 0.92 lightness), and an uncolored pill at
   `oklch(0.74 0.18 243)` (was 0.83).
3. **Intensity lives in the stylesheets.** The keyframes now run the overlay
   from **1** to 0, and each target sets its own strength as the alpha of its
   fill (pill 0.9, tab 0.12). Timing is unchanged: 40 ms hold, 300 ms total,
   100 ms throttle.

## Revision 2 (2026-09-24): owner feedback after testing a dev build

This revision **replaces** §2.1 (routing), the color and envelope parts of
§3.1, §3.2's numbers, and §3.3 (reduced motion). The rest still holds.

1. **Always flash, no routing.** Every tone that passes the gates (§2.2)
   flashes **both** the source's window tab (active or not) **and** its own
   pill in its pane header (whether or not its window tab is showing),
   including for the focused pane. The owner wanted the flash every time,
   regardless of what is on screen. §2.1's table, the focused-pane exception,
   and the `flashTargetFor` routing rule are gone. An event is just
   `{ blockId }`.
2. **Color: a brighter, more saturated version of the pane's own color.**
   The base is the source block's active-border color, resolved the same way
   its frame and pill do (`computeBlockActiveBorderColor`: `frame:hue` first,
   then `frame:activebordercolor`). A window tab falls back to its own
   `tab:color`, then to `--accent-color`. The pill uses its own
   `--pane-tab-underline`, then accent. Both stylesheets brighten that base
   with `oklch(from <base> min(0.92, l + 0.15) c×1.6 h)`. Chromium gamut-maps
   any out-of-gamut result.
3. **Envelope: a click.** The overlay jumps to opacity **0.9** in the flash
   color, holds for **40 ms**, then fades to 0 with `ease-out` over the rest
   of **300 ms**. A tone that lands mid-fade restarts it from peak. The
   per-element throttle is **100 ms**.
4. **Reduced motion no longer changes the envelope.** An opacity fade in
   place isn't motion (nothing moves, scales, or slides), and a fade is the
   usual reduced-motion substitute. The owner's machine has the OS setting
   on, and the no-fade version was not what they asked for.
5. **Photosensitivity rationale updated** (code comment beside the
   constants). Each click is now high-contrast, so the argument is the
   WCAG 2.3.1 small-safe-area exemption rather than low luminance. A tab
   (~200×33 px) or pill (~120×20 px) is a small fraction of the ~341×256 px
   area the general flash threshold starts at.

---

## 1. The problem

Tool-call tones play for **every** agent pane in the window by default
(`notify:tooltones:scope` defaults to `"all"`,
`frontend/app/notification/sound/sound-service.ts:317`), including panes in
tabs you are not looking at. When several agents are working, you hear a
steady stream of tones and have no way to tell which pane or tab they come
from. Nothing on screen changes, because the source is usually in a
background tab.

Reported case (2026-09-23): the owner heard a burst of tones, saw no pane
scrolling anywhere, and suspected a regression. It was not one: another agent
(Agent1) was working in a background tab. The tone system behaved as designed,
but it gave no way to find the source.

## 2. What we want

Each tool-call tone gets a matching **visual pulse on the element that
identifies its source**: a brief, low-contrast highlight that appears
instantly and fades out over about 700 ms. It is peripheral: noticeable at
the edge of your vision, never demanding attention, and never a strobe.

This is the standard way to show background activity in tabbed UIs: the
flash on the owner's tab (like a taskbar flash or an activity dot) is kept
low-contrast and short, and it is paired with the sound instead of replacing
it.

### 2.1 Which element flashes

The flash must point at something you **cannot already see**. Flashing the
tab you are already on tells you nothing about which of its panes made the
sound.

| Where the source pane is | What flashes |
|---|---|
| In a **background** tab | That **tab** in the tab bar |
| In the **active** tab, but not the focused pane | That **pane's header** |
| The **focused** pane, with the window focused | **Nothing.** Its transcript is already in front of you. |
| The focused pane, but the **window is not focused** | That pane's header (you might be looking at another app on a second monitor) |

The rule is a pure function of `(sourceBlockId, activeTabId, tabs' blockids,
focusedBlockId, windowFocused)`. It must live in one place and be unit-tested
as a table (§6).

### 2.2 When it fires

A flash fires **exactly when a tool tone passes its policy gates**: the master
sound switch, `notify:tooltones:enabled`, and `notify:tooltones:scope`.
The hook point is `playToolToneIfAllowed()`
(`frontend/app/notification/sound/sound-service.ts:311-335`). The flash is
emitted right after the scope check at `:318-326`, **before** the
`ctx`/`isAttached()` check at `:329`. That way it still works before the
AudioContext is primed, and at volume 0.

Consequences, all intended:
- Setting the tone scope to "focused only" silences background tones **and**
  their flashes together. The two stay in sync.
- A new `notify:tooltones:flash` setting (default `true`) turns off only the
  visual. It sits next to the existing tool-tone controls in
  `frontend/app/view/settings/sections/sounds-section.tsx` (the section around
  `:211-235`).
- Replaying old history never flashes. Tones only come from live `tool_call`
  events (`frontend/app/view/agent/useAgentStream.ts:746`), so neither the
  tone nor the flash is triggered by a history reload. See §7.3 for a
  pre-existing gap in the tone side of this.

**Open question for the owner:** should the flash also fire when sounds are
**off** (the master switch or `notify:tooltones:enabled` is false), as a
visual-only activity signal? This spec says **no**, to stay strictly the
"complementary indicator" that was asked for. Changing it later means
dropping two early returns and adding a setting; nothing else in the design
depends on it.

## 3. Visual design

### 3.1 The flash

- **Shape:** an overlay across the whole tab (or pane header) surface, not a
  border or dot. A tab is small, and a fill reads at a glance where a 1–2 px
  line does not.
- **Color:** `var(--accent-color)` (`frontend/app/theme.scss:71`), the same
  token as the active-tab top stripe (`frontend/app/tab/tab.scss:72-80`). No
  raw hex/rgba; `color-no-hex` in `.stylelintrc.json` and the theme-token
  rules would reject it anyway.
- **Envelope:** jump to peak opacity **0.22**, hold for **80 ms**, then fade
  to 0 over **600 ms** with `ease-out`. Total length is about 700 ms. The
  instant attack is what makes it read as "that one, just now". The long tail
  keeps it soft.
- **Layering:** a pseudo-element **overlay** with an animated `opacity`, not a
  change to `background`. Two reasons:
  1. User-colored tabs force their background with `!important`
     (`frontend/app/tab/tab.scss:142-155`), so a background-based flash would
     be invisible on exactly the tabs people color to track agents. An
     overlay composites on top of any fill.
  2. `opacity` on its own layer is compositor-only: no layout, no paint of
     the tab contents. This matters because agent-pane typing latency is an
     active performance workstream
     (`docs/specs/TRACKING_AGENT_PANE_BOUNDED_LIVE_WINDOW_2026_09_23.md`),
     and a flash that fires on every tool call must not cost the main thread
     anything.
- **Pseudo-element choice:** `.tab-inner::after` is already the active-tab
  stripe (`tab.scss:72`). Use `.tab-inner::before` for the tab flash. The
  header needs an equivalent overlay that doesn't conflict with its existing
  children (§4.3).
- **Pointer events:** `pointer-events: none` on the overlay. It must never
  intercept a click, a drag start (pragmatic-dnd owns the tab surface), or a
  hover.

### 3.2 Repeated activity: no strobe

An agent can start several tools per second. Each **new** tone re-triggers the
flash, restarting the envelope from peak. Restarting from peak while the
overlay is still bright is only a small step in brightness, so continuous
activity looks like a **steady soft glow** that fades about 700 ms after the
agent goes quiet. That glow is informative in itself ("this tab is busy").

Guard rails:
- **Per-target throttle:** at most one restart per **150 ms** per tab or
  header. Restarts inside that window are dropped.
- **Why this can't strobe:** a full dark-to-peak jump can only happen after
  the previous flash has decayed, which takes at least about 700 ms. That is
  at most about 1.4 full flashes per second, under the WCAG 2.3.1 limit of
  3 per second, and the 0.22 peak opacity is far below the luminance change
  that criterion is about. Record this reasoning in a code comment beside
  the constants.

### 3.3 Reduced motion

The app already has two reduced-motion paths: the `.prefers-reduced-motion`
class (`frontend/app/app.scss:193`, set from `frontend/app/app.tsx:418`) and
an OS-level `@media (prefers-reduced-motion: reduce)` block
(`frontend/app/app.scss:306`). The class path only zeroes `transition-*`, not
`animation-*` or the Web Animations API (the comment at `app.scss:298-305`
explains this), so it won't suppress this flash by itself.

When reduced motion is on, skip the fade. Show the overlay at peak opacity
for the same ~700 ms, then remove it with no animation. The signal (which tab)
survives; only the motion goes. Check this in JS from the same signal
`app.tsx` uses, not only in CSS, because the animation is driven from JS
(§4.2).

## 4. Implementation shape

### 4.1 One signal, emitted from the sound path

Add a small module, `frontend/app/notification/activity-flash.ts`:

```ts
/** Fired once per tool tone that passed its policy gates. */
export function pulseBlockActivity(blockId: string): void;
/** Subscribe; returns an unsubscribe. */
export function onBlockActivity(cb: (blockId: string) => void): () => void;
/** Pure routing rule, §2.1. Exported for the table test. */
export function flashTargetFor(
    sourceBlockId: string,
    ctx: { activeTabId: string; tabBlockIds: Record<string, string[]>;
           focusedBlockId: string | null; windowFocused: boolean },
): { kind: "tab"; tabId: string } | { kind: "header"; blockId: string } | null;
```

`playToolToneIfAllowed()` calls `pulseBlockActivity(blockId)` at the point
described in §2.2, guarded by `notify:tooltones:flash !== false`. Use a
plain subscriber set, not a Solid signal. A signal would make every tab and
header re-run a reactive computation on every tool call. Subscribers compute
their own target and return early when it isn't them.

### 4.2 Tab side

`frontend/app/tab/tab.tsx` already reads the tab object
(`useMuxObjectValue<Tab>`, `:118`), which carries `blockids`. On mount, each
`Tab` subscribes. When `flashTargetFor(...)` returns `{kind: "tab", tabId}`
matching `props.id`, it runs:

```ts
tabInnerEl.animate(
    [{ opacity: 0.22 }, { opacity: 0.22, offset: 0.11 }, { opacity: 0 }],
    { duration: 700, easing: "ease-out", pseudoElement: "::before" },
);
```

The Web Animations API is used instead of toggling a CSS class because it
restarts cleanly on re-trigger. The class-toggle approach needs a forced
reflow to restart a keyframe animation, and that forced layout is exactly
the cost the recent agent-pane performance work removed (PR #3599). Keep the
previous `Animation` handle and `cancel()` it before starting the next one.
Unsubscribe in `onCleanup`.

Styles in `frontend/app/tab/tab.scss`: `.tab-inner::before { content: "";
position: absolute; inset: 0; background: var(--accent-color); opacity: 0;
pointer-events: none; }`. `.tab-inner` must be (or already is) a positioning
context. Confirm the overlay stacks **under** the name text and close button.
A dimmed label would be a regression.

### 4.3 Pane-header side

The generic header is `.block-frame-default-header`
(`frontend/app/block/blockframe.tsx:725`). Agent panes may render hoisted
chrome through `renderPaneChrome` instead
(`frontend/app/tab/pane-leaf-chrome.tsx`, see its header comment and
`docs/specs/SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md`). The
implementer must confirm which element is actually visible as an agent pane's
header and attach the overlay there. Same animation, same throttle, same
reduced-motion rule.

### 4.4 Out of scope for v1

- **Pane-tab stacks:** a source that is an **inactive member** of a pane tab
  stack (the header tab strip, `block-frame-default-header-tabstrip`) is
  hidden inside a visible pane. v1 flashes the stack's header. Flashing the
  specific strip entry is a follow-up.
- **Other windows:** each window runs its own sound service over its own
  panes. Confirm during implementation that a tone is only played by the
  window that renders the pane. If so, the flashing tab is always in the
  window that made the sound, and there's nothing to do across windows.
- **Overflowed tab bar:** the tab bar scrolls horizontally
  (`frontend/app/tab/tabbar.scss:32-44`). A flash on a tab scrolled out of
  view is invisible. A follow-up could flash the overflow edge. Don't
  auto-scroll the bar; that would move things under the user's cursor.

## 5. Acceptance

1. With two tabs, an agent working in tab B, and tab A active: every tool
   call in B produces a tone **and** a visible soft pulse on tab B's tab. Tab
   A never pulses.
2. An agent working in a non-focused pane of the active tab pulses that
   pane's header, not the tab.
3. An agent working in the focused pane, with the window focused, produces
   tones but no flash.
4. With continuous tool calls, tab B shows a steady soft glow, not flicker. It
   fades completely about 700 ms after the last tool call.
5. On a user-colored tab, the pulse is visible over the custom color.
6. `notify:tooltones:scope = "focused"` silences both tone and flash for
   background panes. `notify:tooltones:flash = false` keeps tones and removes
   flashes.
7. With reduced motion (OS setting or the app's own), the pulse appears and
   disappears without fading.
8. Clicking or dragging a tab mid-pulse behaves exactly as it does without
   one.
9. No change in the agent-pane typing benchmark (the CDP scripts under
   `scripts/ui-screenshots/`, `typing-under-load-experiment.mjs`) with two
   background agents streaming tool calls. Measure before and after.

## 6. Tests

- `flashTargetFor`: a table test covering every row of §2.1, plus an unknown
  `sourceBlockId` (a pane in no tab, returns `null`).
- Sound service: extend the tests in
  `frontend/app/notification/sound/__tests__/`. A `tool-started` event that
  passes the gates calls `pulseBlockActivity` once. Master off, tool tones
  off, focused scope with a non-focused source, and `notify:tooltones:flash =
  false` each call it zero times. It still fires when the AudioContext is not
  primed.
- Throttle: three pulses to the same target within 150 ms start one animation.
- Tab: extend `frontend/app/tab/tab.test.tsx` so a pulse for a block in this
  tab's `blockids` calls `animate` with `pseudoElement: "::before"`, a pulse
  for a block elsewhere does not, and unmount unsubscribes.

## 7. Notes

### 7.1 Why not a persistent "busy" dot instead

A persistent dot that stays while an agent is working answers "which tabs
are busy?", which is related but different. This feature answers "what just
made that sound?", and that needs something tied to the moment of the sound.
The steady glow in §3.2 already gives a rough busy signal for free. A real
busy dot, driven by `turnPhase` rather than tool calls, deserves its own spec.

### 7.2 Why not flash the whole pane body

It is louder, it covers transcript text you may be reading, and the typical
source is in a background tab where there is no pane body on screen.

### 7.3 Pre-existing gap noticed while writing this (not part of this change)

`sound-service.ts` has a `replayMode` guard (`:42`, checked at `:172`, `:248`,
`:312`) meant to mute sounds during history replay, but nothing in
`frontend/` ever calls `setReplayMode(true)`. Its only export was removed as
unused in #2407. It is harmless today, because history reload never
dispatches `ToolStart`. If a future change routes replayed history through
the live event path, every past tool call would replay a tone, and with this
spec a flash too. Either wire `setReplayMode` up or delete the dead guard.
Tracked separately.

## 8. What shipped (2026-09-23)

Code: `frontend/app/notification/activity-flash.ts` (routing rule, bus,
`flashElement`), with the hook in `playToolToneIfAllowed()` (sound-service.ts)
and subscribers in `frontend/app/tab/tab.tsx` and
`frontend/app/element/PaneTabStrip.tsx`. Setting: `notify:tooltones:flash`
(Rust `wconfig/types.rs`, `schema/settings.json`, `srv-types.d.ts`,
`settings-template.jsonc`, and a toggle under Settings → Sounds → Tool-call
tones). The §2.2 open question was answered with the recommended default: no
flash when sounds are off.

Departures from §2–§4 as first written (as of Revision 2):

1. **Every tone flashes both targets.** There's no routing (see Revision 2).
   The pill target comes from every pane header now rendering its members
   as pills through the shared `PaneChrome` → `PaneHeaderTabStrip` →
   `PaneTabStrip`
   (`docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md`), one
   pill per blockId. That also covers §4.4's first gap: a background member
   of a pane stack flashes its own pill. `PaneTabStrip` gets an opt-in
   `flashOnActivity` prop, set only by `PaneHeaderTabStrip`, because the
   editor file tabs and the agent History strip reuse the same component with
   ids that aren't blockIds.
2. **Known gap (closed 2026-09-24):** with `pane:tabstrip = "multi-only"`, a
   single-block pane showed no pill, so only its window tab flashed. The
   setting has been removed and every pane now has a pill
   (`SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md` §8).
3. **Overlay under the content:** each host gets `isolation: isolate`, and the
   overlay sits at `z-index: var(--zindex-activity-flash)`, a new token in
   `theme.scss` set to -1, because stylelint only allows z-index tokens. This
   keeps it above the host's background (including the `!important` custom tab
   color) and under the label and close button.
4. **Base color passed inline:** `flashElement(el, baseColor)` sets
   `--activity-flash-base` on the element, and the stylesheet brightens it.
   The tab passes the source pane's color. The pill passes nothing and uses
   its own `--pane-tab-underline`.
5. **Per-keyframe easing**: the hold segment is linear and the decay uses
   `ease-out`.

Tests: `frontend/app/notification/__tests__/activity-flash.test.ts` (the bus,
the envelope, the base-color variable, and the throttle), a new "activity
flash" suite in `sound/__tests__/sound-service.test.ts` (one event per tone,
including from the focused pane, plus every gate, with the player unprimed),
and flash suites in `tab.test.tsx` (active and background tabs, pane color)
and `PaneTabStrip.test.tsx`. Revision 2 was also checked live in a dev build
over CDP: an amber (`#f59e0b`) pane flashed its pill and its window tab with
`oklch(0.92 0.26 70)`, an uncolored pane flashed with a boosted accent,
`oklch(0.83 0.21 243)`, and both overlays were back at opacity 0 within
400 ms. Acceptance items 1–8 in §5 are for manual
verification in a dev build. Item 9 (the typing benchmark) was not run for
this change.
