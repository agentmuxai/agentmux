# SPEC — Browser pane: stop the loading-brain flicker / page-hide flashing

**Date:** 2026-08-17
**Type:** Analysis + architecture proposal (root-caused via code investigation — no code shipped yet)
**Status:** Draft
**Scope:** `agentmux-cef/src/client/navigation.rs`, `agentmux-cef/src/browser_pane/callbacks.rs` (Rust, CEF LoadHandler wiring) + `frontend/app/view/browser/browser-model.ts`, `browser-view.tsx`, `frontend/app/store/browser-pane-state/reducer.ts` (frontend loading-state plumbing and spinner rendering).

## Problem

On a browser pane (any `view: "browser"` pane, including the messenger
widgets), the "pulsating brain" loading indicator (`BrainSpinner`) flickers
repeatedly during and after a page load: it flashes, briefly reveals the
already-rendered page for a couple hundred ms, then flashes back to the
spinner, sometimes several times, before finally settling on the page.
This reads as broken/janky rather than "the page is loading."

## Root cause — two independent, compounding mechanisms

This bug has **two separate causes stacked on top of each other**. Neither
alone would look this bad; together, one produces a real but harmless
extra state-signal, and the other turns every one of those signals into a
hard, visible full-pane hide/reveal.

### Cause 1 — the loading signal isn't scoped to the main frame

`model.loadingAtom()` (`frontend/app/view/browser/browser-model.ts:118-120`),
which drives the spinner, is set directly from CEF's `is_loading` flag,
forwarded verbatim end-to-end:

```
CEF LoadHandler::OnLoadingStateChange(browser, is_loading, can_go_back, can_go_forward)
  → agentmux-cef/src/client/navigation.rs:301-320 (AgentMuxHandler::on_loading_state_change)
  → agentmux-cef/src/browser_pane/callbacks.rs:435-478 (on_loading_state_change_browser_pane)
      emits `browser-pane-nav-state` IPC event with is_loading verbatim
  → browser-model.ts:369-432, `is_loading !== undefined` branch (line 412-425)
      dispatches TabLoadingChanged with loading: payload.is_loading directly
  → reducer.ts:376-416 TabLoadingChanged — dedups only against the immediately
      prior value, no "is this a real top-level nav" check, no dwell time
  → loadingAtom → BrainSpinner mount/unmount (browser-view.tsx:91-115)
```

The problem: `OnLoadingStateChange`'s CEF signature carries **no frame
parameter at all** (confirmed against the vendored `cef` crate binding) —
it reports on `WebContents`-level aggregate loading state across the
*entire frame tree*, not the main document alone. Any sub-frame load —
an `<iframe>`, an ad refreshing, a chat/analytics widget, a lazy-loaded
embed, all extremely common on real sites — is a genuine Chromium-level
frame navigation and can flip this aggregate flag `true → false → true`
well after the main page has visibly finished rendering. Nothing in the
pipeline above filters this to "the main document's own load," because the
callback that carries `is_loading` structurally can't distinguish which
frame triggered it.

This is a **new finding**, not a regression of a previously-fixed bug.
`docs/specs/SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11.md` (the
spec that built this wiring) fixed a different, real bug — `loading` was
being cleared within the same tick it was set, so the spinner never showed
at all (§2, §4.1-4.2) — and that fix is correctly implemented in the
current code. That spec's §4.4 ("Deliberately not a concern") only reasons
about SPA client-side routing (correctly ruled out — `pushState` doesn't
trigger LoadHandler callbacks) and explicitly *flags but accepts* multi-hop
OAuth-redirect flicker as a minor, known cost. It never considered
sub-frame/subresource aggregation, and its own §6 open question 3 flags the
overlay mechanism below as unverified at scale — an open risk the spec's
author left for follow-up, not something ruled out.

The codebase already has the pieces needed to filter this correctly, just
not wired together for this purpose: `on_load_start`
(`client/navigation.rs:343-359`) and `on_before_browse`
(`client/lifecycle.rs:794-824`) both carry a real frame reference and are
already filtered to `frame.is_main()` — currently only to arm/disarm the
pane's load-timeout watchdog (`arm_pane_load_watchdog`/
`update_pane_load_watchdog_url`, `browser_pane/callbacks.rs:58-102`), not
to gate the loading signal sent to the frontend.

### Cause 2 — the spinner overlay is full-pane-size, so every flip is a hard native-window hide/show, not a harmless redraw

Independent of *why* `is_loading` flips, the mechanism that renders the
spinner amplifies every flip into something far more disruptive than a DOM
redraw. `.browser-loading-overlay` (`browser-view.tsx:189-193`) is sized to
cover the entire `.browser-placeholder` — 100% of the pane's rect — and is
tagged `data-pane-overlay`. On Windows, mounting/unmounting that element
round-trips through the real airspace-clip mechanism
(`pane-overlay-auto.ts` → `pane-overlay.ts:83-207` →
`browser_panes_set_overlay_clip` IPC →
`agentmux-cef/src/browser_panes/clip.rs`'s `SetWindowRgn`), which performs
a **hard, non-animated, full native-HWND hide** — not a spinner drawn over
an already-loaded page, but the *entire live pane* being clipped out of
existence, then restored, at the Win32 level. Chromium keeps compositing
behind the clip the whole time, so restoring it instantly reveals whatever
was already rendered.

So: cause 1 produces a spurious `is_loading` flip; cause 2 turns that flip
into "hide the whole visible page, then show it again" instead of a
harmless spinner blip over already-rendered content. This combination —
called out as an explicit open risk in
`SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11.md` §3 and §6.3
("first use of `data-pane-overlay` at 100% pane coverage... worth a manual
perf/correctness pass... before considering this done") — is what produces
exactly the reported symptom: "flash into multiple loads, show a page for
a couple ms, then go back to the pulsating brain."

### Ruled out

- **Pane HWND destroy+recreate on resize/tab-switch/focus** — not found.
  `usePaneRectSync` only calls `browser_pane_resize` (a `SetWindowPos`
  reposition of the existing HWND); `createPane()` only fires once, before
  the pane exists. A genuine tear-down+recreate exists only for
  cross-window "redock" (dragging a pane to another window) — a narrower,
  drag-specific trigger, not a match for in-place loading flicker.
- **Duplicate `BrowserViewModel` construction** (double `navigate()` calls)
  — `block.tsx`'s view-model-construction effect is keyed on `meta.view`
  only, which an ordinary nav-state/URL write never touches, so a second
  view-model isn't constructed on a normal page load. (The `__diagVmId`
  tracking in `browser-model.ts:205-212` exists because this class of bug
  *has* occurred historically for other reasons — worth checking
  `muxlog`'s `vm=` tags during a live repro to rule it out with certainty,
  but nothing in the current construction path explains it for this
  symptom.)

## Design — three complementary layers, not alternatives

Each layer independently reduces the blast radius; together they close
this out completely. Recommend implementing all three — they're small,
independent changes, not competing redesigns, and layer 2 is worth having
even after layer 1 ships (defense in depth against whatever cause 1 misses
— a genuine OAuth-redirect chain, for instance, is *real* top-level
navigation churn that layer 1 will not and should not suppress).

### Layer 1 (root cause, highest priority) — scope the loading signal to the main frame

Stop trusting `OnLoadingStateChange`'s raw, frame-blind `is_loading` as the
spinner's source of truth. Instead, derive the emitted `is_loading` from
the pane's existing main-frame navigation state machine — the same
arm/disarm tracking already built for the load-timeout watchdog
(`arm_pane_load_watchdog` on `on_before_browse`'s main-frame branch,
disarmed on `on_load_start`'s main-frame branch — `browser_pane/callbacks.rs:58-102`,
`client/lifecycle.rs:794-824`, `client/navigation.rs:343-359`). That state
is already exactly "is this pane's main document currently between
navigation-start and navigation-committed/finished" — precisely what the
spinner needs, and it structurally cannot be perturbed by a sub-frame or
subresource load, because those never touch `frame.is_main()`-gated code.

Concretely: `on_loading_state_change_browser_pane` continues to forward
`can_go_back`/`can_go_forward` (those are legitimately browser-level, not
frame-specific, and the back/forward buttons already consume them
correctly) but stops forwarding its own `is_loading` parameter for
spinner purposes. The `is_loading` field in the `browser-pane-nav-state`
payload instead reflects the watchdog's main-frame-armed state, updated
wherever that state already transitions (main-frame `on_before_browse` →
true, main-frame `on_load_start`/`on_load_end` completion or `on_load_error`
→ false). This unifies "is the pane's main document loading" into one
source of truth consumed by both the watchdog and the spinner, rather than
two different signals (frame-blind `is_loading` for the spinner,
frame-scoped arm/disarm for the watchdog) that happen to usually agree.

No frontend reducer change needed for this layer — `TabLoadingChanged`
still receives a plain boolean, just a more accurate one.

### Layer 2 — don't let a loading-state flip on an already-painted pane blank the whole page

Even after layer 1, a real main-frame loading cycle can still happen more
than once in quick succession for a legitimate reason (redirect chains,
explicit reload/back/forward) — §4.4 of the original spec already accepted
this. The fix here is not to suppress those flips, but to stop letting
each one cost a full-pane hide when there is no longer a real "gap" to
cover.

Split the spinner's overlay behavior by whether the pane has ever
successfully painted content yet:

- **First load after pane creation** (nothing has rendered yet — hiding
  the HWND costs nothing because there's nothing behind it to hide):
  keep today's full-pane `data-pane-overlay` behavior. This is the
  original, correctly-motivated use case from
  `SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11.md` (covering
  Chromium's own blank-page gap while a heavy SPA bundle boots) and should
  be left exactly as-is.
  - Also see `docs/specs/SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md`'s own
    freeze-frame coordination (`pane-overlay.ts:172-203`, `wait()` on the
    `pane-overlay-clip-changed` event) — that machinery already exists for
    letting a pane's snapshot paint under a hide before the native hide
    lands, avoiding a bare-placeholder flash. It is not currently invoked
    for this component; confirm during implementation whether wiring
    `useFreezeFrame`'s snapshot into this specific hide (instead of the
    plain placeholder background) further smooths even the legitimate
    first-load case.
- **Any subsequent loading flip on a pane that has already painted once**
  (reload, back/forward, a second real navigation, or anything layer 1
  didn't catch): render a small, non-full-pane loading affordance instead
  — e.g. a corner/edge badge or a thin top progress bar, sized so its
  `data-pane-overlay` rect does **not** cover the area where the
  previously-rendered page is visible. A flip here no longer has anything
  to hide, so it can't produce the "page flashes away and back" effect
  regardless of how many times it happens.

This requires the view to track a simple `hasPaintedOnce` signal (set true
on the first `TabLoadingChanged(loading:false)` after pane creation, never
reset) and branch the `<Show when={spinnerMounted()}>` block in
`browser-view.tsx:189-193` on it.

### Layer 3 (cheap insurance) — minimum-dwell debounce in the reducer

Add a small minimum-visible-duration / coalescing window (~150–250ms) to
`TabLoadingChanged` handling so that a `true` immediately followed by a
`false` (or vice versa) within that window collapses into a single visible
transition instead of a double-flicker. This is pure defense-in-depth —
layers 1 and 2 address the actual causes — but it's cheap, catches
anything neither layer anticipated (e.g. a same-tick redirect hop), and is
the same class of fix already used elsewhere in this codebase for exactly
this kind of rapid-signal problem (`pane-overlay.ts`'s own rAF-coalescing
of overlay-rect changes, `pane-overlay.ts:54-93`). Should not be treated as
a *substitute* for layers 1–2 — masking symptom without fixing signal
fidelity would still let a chatty page (ads that keep re-triggering
sub-frame loads for tens of seconds) wear through the debounce window
repeatedly.

## Scope / non-goals

- Does not touch the status-bar popover airspace bug — unrelated,
  covered in `docs/specs/SPEC_STATUS_BAR_POPOVER_AIRSPACE_CLIP_2026_08_17.md`.
- Does not change `BrainSpinner`'s own visuals or the existing 200ms
  CSS fade — only when/how often it mounts, and how much of the pane its
  overlay claims.
- Does not remove or weaken the load-timeout watchdog — layer 1 reuses its
  state, doesn't alter its own arm/disarm/timeout behavior.
- Out of scope: macOS/Linux verification of layer 2's badge rendering —
  the airspace-clip mechanism this bug lives in is Windows-specific
  (`pane-overlay-auto.ts:130`), so layers 1 and 3 matter on all platforms
  but layer 2's "avoid clipping the painted area" concern is a Windows-only
  concern by construction; non-Windows behavior should be spot-checked but
  is not expected to need platform-specific code.

## Open questions

1. **Layer 1 exact state shape** — this spec proposes reusing the
   watchdog's main-frame arm/disarm tracking as the loading signal's
   source, but the exact field/struct backing that state
   (`browser_pane/callbacks.rs:58-102`) needs to be read in full before
   implementation to confirm it can be safely read from
   `on_loading_state_change_browser_pane`'s call site (or whether
   `is_loading` should instead be emitted directly from the arm/disarm
   transition points themselves, bypassing `OnLoadingStateChange` for this
   purpose entirely — likely the cleaner design, since it removes the
   frame-blind callback from the spinner's path altogether rather than
   correlating against it).
2. **Layer 2's "small affordance" design** — a corner badge vs. a top
   progress bar vs. reusing `BrainSpinner` at a smaller size are all viable;
   left to implementation/design judgment, not prescribed here.
3. Should `hasPaintedOnce` reset on an explicit user-initiated reload (so a
   manual refresh gets the full "friendly gap cover" treatment again), or
   only on true pane recreation? Leaning toward "only on recreation" —
   reload of an already-visible page has nothing worse to show the user
   than the small affordance — but flagging as a product call, not
   dictating it here.
4. Verify whether `useFreezeFrame`'s existing snapshot mechanism
   (`use-freeze-frame.ts`, not read in full during this investigation)
   already solves part of layer 2 for free, or is scoped only to the
   redock/drag case — needs a read before implementation.

## References

- `docs/specs/SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11.md` —
  the spec that built the current (partially correct) wiring; fixed a real
  same-tick-clear bug, but never investigated sub-frame aggregation and
  explicitly flagged the full-pane-overlay risk as unverified (§3, §6.3).
- `docs/specs/SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md` — the airspace-clip
  mechanism whose full-pane-size use is cause 2.
- `frontend/app/view/browser/browser-model.ts`,
  `frontend/app/view/browser/browser-view.tsx` — frontend loading-state
  plumbing and spinner rendering.
- `frontend/app/store/browser-pane-state/reducer.ts` — `TabLoadingChanged`
  handling, the natural home for layer 3's debounce.
- `agentmux-cef/src/client/navigation.rs`, `agentmux-cef/src/client/lifecycle.rs`,
  `agentmux-cef/src/browser_pane/callbacks.rs` — CEF LoadHandler callbacks,
  the existing main-frame-scoped watchdog state layer 1 proposes reusing.
- `docs/reports/REPORT_BROWSER_PANE_GOOGLE_LOGIN_INSTANCE_EXIT_AND_UAC_2026_08_11.md` —
  unrelated bug, but adjacent: also flags the airspace/overlay subsystem's
  lifecycle-visibility fragility as worth a broader look.
