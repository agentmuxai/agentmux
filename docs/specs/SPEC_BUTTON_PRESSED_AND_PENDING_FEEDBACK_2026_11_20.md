# Spec: Every button answers the press at once and shows a spinner while its action runs

**Status:** implemented in part (PR A, the shared Button; PR B, the sweep, and PR C, the guard, follow), 2026-11-20. Task #48232 (spec), build #48233. Author: Agent1@narko.
Related: #48231 (minimize delay, Loap-3), `SPEC_PANE_CLOSING_INDICATOR_AND_FASTER_CLOSE_2026_10_28.md` (the close X).

## 1. Requirement

When a person presses any button in AgentMux, the button answers in the same frame: it looks pressed, it cannot be pressed a second time, and if the action it started is still running a moment later it shows a spinner until the action is done. A button never sits unchanged while something happens behind it, and a slow action is visible as slow instead of looking like a dead control. A failed action ends the pending state and is reported the way it is today.

## 2. How the close X does it today (read from the code, 2026-11-20)

The X on a pane is not a general mechanism. It is one hand-built flow:

| Step | Where | What it does |
|---|---|---|
| Click | `layoutMagnify.ts` `closeNode`, `layoutStack.ts` `closeBlockInStack` | Calls `beginClosing(model, nodeId, tabs)` **before its first `await`**, so the next frame already shows it. A second close of the same pane does nothing. |
| State | `layout/lib/layoutClosing.ts` | Two signals beside the layout model: node id to tab count, and block ids for one tab. `isClosing()` and `closingTabs()` read them. `endClosing` runs in a `finally`, on "gone" and on "kept". |
| Look | `block/closing-label.tsx`, `blockframe.tsx` | The header's end buttons are replaced by a spinner (`fa-spinner fa-spin`) and "Closing…" (or "Closing 5 tabs…"). No delay: the spinner shows from the first frame. |
| One tab | `PaneTabStrip.tsx` ~615 | The pill's X is a `CloseButton` with `busy={p.closing()}`: the glyph becomes the spinner and the button is disabled. |
| The button itself | `element/ui/CloseButton.tsx` | `busy` swaps the ✕ for `fa-spinner fa-spin`, sets `aria-busy`, and disables the button. It is the only control with an `:active` pressed style (`close-button.scss`). |

What this tells us: the X is quick to show feedback because its **owner holds a "this is happening" flag and sets it synchronously**; the button only draws it. Nothing in the app does that for you. The general version has to put the flag in the button, driven by the one thing every async click handler already has: the promise it returns.

The ui kit's `Button` already has `busy` (spinner replaces the icon, disables the button, `aria-busy`), and about 70 call sites turn it on by hand with their own signal (`busy={busy()}` plus `onClick={() => void act()}`). `IconButton` cannot take it at all: its props omit `busy`.

## 3. Reach (measured with the type checker, not by text search)

A one-off, read-only script over `tsconfig.citypecheck.json` classified every `onClick={…}` on a JSX element in non-test source (1,099 of them):

| Element | onClick handlers | Return a promise | `() => void x()` where `x` is async (promise thrown away) | Already pass `busy` |
|---|---|---|---|---|
| `Button` / `IconButton` (893 uses in 307 files) | 893 | 38 | **272** | 70 |
| raw `<button>` | 17 (12 files) | 1 | 1 | 0 |
| other components taking `onClick` | 65 | 3 | 3 | 1 |
| other lowercase elements (`div`, `span`, …) | 124 | 4 | 3 | 0 |

Three corrections to the first reading of the task:

- **Raw `<button>` is small.** The 125 in 71 files includes tests. Non-test source has about 23 (12 files with a handler), and 7 of those are the kit's own primitives (`Button`, `CloseButton`, `Tag`, `Tabs`, `SegmentedControl`, `Swatch`, `inputs`). The rest are the OS-style window controls, `StatusBar`, `logo-menu`, `markdown-media`, `SystemStats`, `TokenUsage`, `PaneTabStrip`, `browser-nav-parts`, `MemoryTile`, `MemoryFileCard`.
- **A change in the shared `Button` does reach everything**: the legacy `element/button.tsx` is a wrapper over the kit's `Button` (15 files import it by that path; the rest use the kit directly), so 893 of 893 go through one component.
- **But a promise has to be visible to be waited for.** Almost 9 in 10 of the buttons that run something async (272 of 310) throw the promise away with `void`, so a `Button` that only waits for a returned promise would show nothing on them. The sweep (§8) turns those into returns.

Of the 310 async buttons, **60 already have a hand-wired `busy`** and about **250 show nothing**. Most of the 250 are quick (open a link, copy, reveal a file, switch a view) and must never flash a spinner. The slow ones that matter, by what they do:

- Updates and restarts: `RestartToUpdatePopover` (upgrade and restart, cancel), `maintenance-parts` (cancel restart, reveal).
- Installs and tool runs: `SystemToolInstallInline`, `ProviderUpdateInline`, `ProviderUninstallInline`, `create-panel-install-view` (start, cancel), widgets (`widgets-section-parts`: install, rescan, enable, remove, key trust), `widget-catalog`.
- Decisions that go through srv: vault (`vault-use-question`, `vault-row-asks`, `vault-requests-list`), desktop and widget-install requests (`desktop-request-card`, `desktop-requests`, `widget-install-card`, `agent-notice-card`), browser attention (`browser-attention`, `driven-by-strip-parts`: take over, approve), the Kanban drawer (`m.act` and `takeBack`: start, accept, send back, retry, cancel; about 20 buttons).
- Saves and sends: `AgentSpendCard`, `AgentModelUndoneCard`, Swarm broadcast "Send", Swarm start and open agent, `GlobalMemoryFullView`, native memory save, skill import and commit, Files (`FilesToolbar` new folder, `FilesFooter` cancel and undo, `FilesPlaces` navigate), remotes, Tower, History.
- Session actions: archive, restore, export (`AgentSessionStats`, `AgentSessionNotices`; these have their own `busy` signals but through a `run(...)` helper, not the button).

A handful return a promise that waits for a **person**, not for srv (`GlobalMemoryFullView` ~172 and `native-memory-manager` ~892 await a confirm panel; `requestCancel` handlers). A spinner while a person reads a question would be wrong; these opt out (§5).

## 4. The mechanism

All of it lives in the kit's `Button` (`element/ui/Button.tsx`), so the legacy wrapper, `IconButton`, `DeclIconButton` and every call site get it with no edit.

1. **Pressed, at once, for every button.** `.ui-button:active:not(:disabled)` uses the same `pressed-look` the `aria-pressed` state already has, with `transition-duration: 0s` for the press so it is the same frame as the mouse-down, and a normal release. The `chrome` variant (pane header icons) has no pressed look today; its press is a drop in opacity to the disabled step and back. Keyboard: `Space` and `Enter` already trigger `:active` on a button.
2. **Pending, from the handler's result.** `Button` wraps `onClick`: it calls the handler, and if the result is a promise (`typeof r?.then === "function"`) it sets `pending` **synchronously, in the same call**, and clears it when the promise settles.
3. **While pending**: further clicks do nothing (`aria-disabled` and the handler ignored; not `disabled`, so the button keeps its look and its focus, and keyboard focus is not lost to a vanished element); `aria-busy="true"`.
4. **The spinner appears after a short delay** (`PENDING_SPINNER_DELAY_MS`, 120 ms). Until then the button is pressed-and-locked but unchanged, so an action that finishes inside 120 ms never flashes a spinner. After it, the spinner shows in the button (below).
5. **Failure.** The wrapper settles with `.then(clear, (e) => { clear(); report(e); })`, where `report` rethrows the error asynchronously (`queueMicrotask(() => { throw e })`) so it reaches the same place an unhandled rejection reaches today (`log/error-forwarder.ts`, the console). It does not add a toast or hide anything. A handler that already catches and shows its own error keeps doing that and the button simply clears.
6. **`busy` stays.** A caller that already passes `busy` keeps working: the spinner shows when `busy || (pending && delayElapsed)`. Nothing has to be removed; the 60 hand-wired ones can drop their signal later (D3).
7. **Event handler forms.** Solid's `onClick={[fn, data]}` form is handled too.

### The spinner, without a layout jump

- **With a leading icon**: the spinner replaces the icon at the same size (what `busy` does today).
- **Label only**: the spinner is centred over the button and the label stays in the flow with `opacity: 0`, so the button keeps exactly the width the label gave it. Nothing is reserved at rest, so no button in the app gets wider; the label comes back when the action ends. (The alternative, a reserved slot before the label, would widen about 600 buttons that never go pending.) The layout test below checks the box does not change between resting, pending and spinning.
- **Icon only** (`IconButton`, `chrome`): the spinner replaces the glyph in the same box.
- **Layout test**: a real-browser test measures the button's box before the press, while pending without the spinner, and with the spinner, for each variant; the width and height must be equal.

### Spinner now or after a delay: the trade-off

| | Spinner at once (like the X) | Spinner after 120 ms (recommended) |
|---|---|---|
| Quick action (copy, open a link, navigate; most of the 250) | A one or two frame flash on every press. It reads as a glitch and teaches people to ignore the spinner. | Nothing but the press. The button looks done as fast as it was. |
| Slow action | Shows from the first frame. | Shows 120 ms in, which is under the point (about 100 to 200 ms) where a person starts to feel a wait; and the press already told them it registered. |
| Double submit | Prevented. | Prevented the same way: the lock is immediate, only the spinner waits. |
| Closing a pane | Needs it at once: the pane is about to vanish and `Closing…` replaces its buttons. | The close keeps its own immediate state (§6). |

Recommendation: **pressed and locked at once for every button; the spinner after 120 ms.** The X keeps showing at once because it does something different (it replaces the header), and that stays.

## 5. Opt-outs

A prop `pending?: false | "lock"` on `Button` (default: automatic). Defaults by kind, so most cases need no edit:

| Control | Default | Why |
|---|---|---|
| Steppers, increments, nudge buttons, "repeat while held" | **Opt out** (`pending={false}`): never locked, no spinner | They are meant to be pressed again before the first has finished. |
| Toggle buttons (`pressed` is set) and tabs | Pressed look from `pressed` only; no spinner by default. A slow toggle can pass `busy` | The result is the new state; a spinner on a switch in the middle of changing reads as a second state. |
| `row` and `link` variants, menu entries | Lock and press only; the spinner shows only if the menu or popover is still open after the delay | A menu entry normally closes its menu; the menu going away is the feedback. |
| `chrome` icons (pane header, search bar) | Pressed look at once; spinner replaces the glyph after the delay | Same as an icon-only button. |
| Icon-only buttons in a tiny box (the 18 px `sm` ones) | Spinner replaces the glyph, at the glyph's size | No label to protect. |
| A handler that waits for a person (an inline confirm, a question) | **Opt out** (`pending={false}`) | The promise is a human's answer, not work. The 3 found today are listed in §3. |
| Window caption buttons (minimize, maximize, close) | **Not the kit's Button**: they mimic the OS; they keep their own markup | The OS draws the result. Close already has its closing state. See §6 for minimize. |

## 6. How this relates to the slow-action work (#48231 and the close)

The risk the operator named is real: a spinner makes a slow button look *working* when it should have been fast. Three guards:

- **The pane close stays as built.** It sets its flag before its first await and replaces the header; it does not go through the button's pending state (the pane's buttons are swapped out), so nothing changes there. `CloseButton` already takes `busy`; the shared mechanism applies to it for the cases that are only a plain close (a drawer's X).
- **Minimize is not helped by this, on purpose.** The caption buttons are the OS's. A spinner on minimize would sit on a button whose result is the window going away; the fix for the delay is Loap-3's #48231 (the delay itself). Nothing in this spec touches it.
- **A button trace shows the slow ones.** Like `__closeTrace()`: a ring of the last 50 presses that stayed pending more than 300 ms, read from the console as `__buttonTrace()`: label (the button's accessible name), the `data-testid` if any, the call site's component name if given, the time pending, and whether it failed. A press pending more than 1 s logs one console line. This is the list of actions to make fast, produced by use instead of guessed; it is how we find the 250's slow members without a lab.

## 7. Accessibility, motion and themes

- **`aria-busy="true"`** while pending (already how `busy` renders). The button keeps focus; a locked button is `aria-disabled`, not removed or `disabled`, so a screen reader or keyboard user is not thrown off it.
- **No announcement spam**: a live region would announce every press. Only a button pending longer than 1 s adds a visually hidden "Working" text node (`role="status"`), cleared on finish.
- **Motion.** The spinner is the kit's `fa-spin`, which the app's motion policy already governs (`app.scss`: slowed in the reduced tier, 1 ms in the off tier). In the off tier it would show as a still glyph; the spec keeps the still glyph (it still says "busy") and the `aria-busy`. The press is a colour step (no motion), so it needs no reduction. Under the Animations preset nothing here is an animation of its own, so no new entry.
- **Contrast.** The spinner is `currentColor`, the button's own text colour, which the tone tests already hold at 4.5:1 on its surface in every theme (`tag-contrast.test.ts` and the tone checks). No new colour. The pressed look reuses `pressed-look`, already measured.
- **Zoom.** Sizes come from the `--ui-*` density tokens, so chrome zoom scales the slot with the button; the layout test runs at 0.5, 1 and 1.5.

## 8. Build split (#48233)

1. **PR A: the shared button.** `Button` pending mechanism, pressed state, spinner slot, `IconButton` accepts `busy`, `pending` prop, `__buttonTrace()`, tests, `CloseButton` reusing the same pending for a plain close. No call site edited. Visual: a look check.
2. **PR B: the sweep.** Turn `() => void x()` into `() => x()` for the async actions that matter (the §3 list, about 120 of the 250; the quick ones such as `openLink` and clipboard stay as they are: they finish inside the delay either way, and changing them adds nothing but risk), add `pending={false}` to the 3 confirm-waiters and any stepper found, and give `IconButton` callers their `busy` where they had a signal. Split by folder (vault, Kanban drawer, settings and toolchain, statusbar and updates, agent pane) so each is reviewable and none touches another agent's file unannounced.
3. **PR C: the guard.** Extend `scripts/check-ui-primitives.mjs` (it already counts the ✕ controls that are not `CloseButton`) with a rule: a raw `<button>` outside `element/ui/` and the OS-style window controls fails, with a baseline of the 12 files today; and a rule that flags `onClick={() => void …}` on a `Button` whose callee is async in a new file. Convert the raw `<button>`s that are not window controls.
4. **Out of scope**: the 124 clickable `div`/`span` and the 65 other components; a clickable `div` that acts as a button is a different cleanup (it should be a `Button`).

## 9. Tests

- **Unit (jsdom, fake timers)**: pending is set in the same call as the click (checked before any await); an action that settles in 50 ms never renders the spinner; one that runs 200 ms renders it at 120 ms and not before; a second click during pending runs nothing; a rejection clears pending, reaches the error path once, and the button is usable again; a sync handler never goes pending; `busy` passed by a caller still wins; `pending={false}` and the toggle default skip the mechanism; the `[fn, data]` handler form works.
- **Layout (real browser, `layout-browser-input.ts`)**: for `default`, `row`, `link`, `chrome`, icon-only and label-only, at three zooms and two densities, the box is the same size before, while pending without the spinner, and with the spinner.
- **Press**: in a real browser, a mouse-down on a button sets `:active` and the computed colours change in that frame.
- **Trace**: ring size, the 300 ms and 1 s thresholds.
- **Sweep**: one test per swept folder showing that the first press locks the button and a second press does nothing while the stubbed RPC is unresolved.
- **Guard**: the rule's own test with a bad and a good fixture.

Look check (visual change, so it is required before merge): macOS (Clare, or Masty) and Linux (Maricon), light and dark, the five tones (accent, neutral, danger, quiet, attention), the `row`, `link` and `chrome` variants, a label-only button and an icon-only one, at zoom 1 and 1.5, in three states each: resting, pressed (real mouse down), pending with the spinner (a slow stub). Each says pass or fail per item with a screenshot of anything wrong.

## 10. Decisions for the operator

- **D1. Spinner timing.** Pressed and locked at once for every button, spinner after 120 ms (recommended), or the spinner at once on every button as the X does. The X itself keeps showing at once either way.
- **D2. Which sites the sweep covers.** The slow ones only, about 120 buttons in five folders (recommended), or every `void` handler, 272, including the quick ones that finish inside the delay and gain nothing.
- **D3. The 60 hand-wired `busy` signals.** Leave them (recommended: they keep working and can go one by one), or remove them in the sweep.
- **D4. The guard.** A rule so new raw `<button>`s and new thrown-away promises on a `Button` fail the primitives check (recommended), or leave it to review.
- **D5. `__buttonTrace()`.** Include it so slow buttons show themselves (recommended), or leave it out of v1.

## 11. Not in scope

Making any slow action fast (the minimize delay is #48231; the trace names the others); the pane close, which keeps its own state; clickable non-button elements; a global busy cursor.
