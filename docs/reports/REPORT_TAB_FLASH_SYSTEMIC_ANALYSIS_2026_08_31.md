# Why the tab flash keeps coming back — a systemic analysis

**Date:** 2026-08-31
**Author:** AgentY
**Status:** Analysis, written while the tab-close flash was still open and
**revised after it was fixed** by PR #2818 (§§8-9 of
`SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md`). The outcome is recorded in
§0 below and is the single most useful thing in this document — it is a
controlled test of the thesis. Companion design:
`SPEC_TAB_WINDOW_RENDER_ARCHITECTURE_2026_08_31.md`.
**Trigger (at time of writing):** After PR #2811 landed four stacked
root-cause fixes (§§2-3, §5, §6, §7), the repo owner reported the flash was
**still present**, on the same gesture: closing a tab through the confirm
modal.

---

## 0. Outcome — what actually fixed it, and what that proves

**Fixed by PR #2818** with two changes, neither of which was another ordering fix:

- **§8 Optimistic removal.** The tab leaves the strip the instant the confirm
  modal opens; cancel or RPC failure puts it back. *"A tab that is not rendered
  cannot flash, whatever order the backend's updates arrive in."* The strip
  stops depending on backend update ordering at all.
- **§9 Targeted reveal gate.** `holdRevealGate(targetTabId)` now names its
  destination, and `workspace.tsx` hides only that tab once it becomes active —
  instead of gating "whoever is active right now."

**Scorecard against this report's predictions:**

| Claim | Verdict |
|---|---|
| The flash is architectural, not a bug with a root cause | **Confirmed** — the fix that worked changed the architecture of the strip, not an ordering |
| Suppressors (gates/debounce/`batch()`) can't close this class; only making the bad state unrepresentable can | **Confirmed** — §8 is verbatim that move, and its own text says "make the flash structurally impossible" |
| §3.5 — the reveal gate is keyed on `tid === tabId()` and is one reactive step behind what it gates | **Confirmed and fixed** — this is exactly what §9 repaired; the real symptom was the *destination* tab's pane blanking toward the 800ms cap |
| §3.3 — the native pane compositor (`sendClip` → rAF → async HTTP) is the leading residual cause | **Not the cause here.** Plausible mechanism, wrong ranking. It remains a real unsynchronized seam (§3.3), with documented precedent, but it was not what the owner was seeing |

Two things I got wrong, recorded so the next pass doesn't repeat them:

1. **I ranked a novel hypothesis above a defect I had already confirmed.**
   §3.5 was verified by code reading; §3.3 was inferred. I led with §3.3
   because it was a better *story* — it explained the "after the modal closes"
   trigger. The confirmed defect was the real one.
2. **A verification gap was doing more work than any code defect.** Per §8's
   own writeup, *none* of §§5-7 was ever tested against a build containing all
   of them at once — v0.55.25 predated the merge, and the dev instance had only
   §§3-6. Some "still broken" sightings were of incomplete builds. Four fixes
   were reasoned about without a single clean observation. That is the
   strongest argument in this document for the "measure first" discipline in §5.

---

## 1. The thing that should worry us

Four consecutive fixes, each one correctly root-caused, each one verified by
tests, each one landing a real defect — and the symptom did not move.

That is not four bad diagnoses. Re-reading them, all four were *true*:

| § | Defect found | Real? |
|---|---|---|
| §2-3 | Close-button click bubbled to the tab's `onSelect`, racing `SetActiveTab` against `CloseTab` | Yes — verified, tested |
| §5 | Client pre-selected the neighbor via a second RPC, splitting one atomic transition into two round trips | Yes — verified, tested |
| §6 | `updateWaveObjects` applied a multi-object response as N unbatched Solid writes | Yes — verified, tested |
| §7 | Two WS push paths delivered the same pair unbatched, and *always arrived before* the response body §6 fixed | Yes — verified, tested |

When four sequential true root causes don't fix a symptom, the symptom is not
"a bug with a root cause." It is **an emergent property of the architecture** —
the system has enough independent, unsynchronized paths to the screen that
closing any one of them just promotes the next one to visibility.

Each fix was a correct answer to "which path painted first *this time*."
None of them addressed "why is there more than one path at all."

## 2. The historical pattern

This is not a two-week-old problem. A scan of the repo's own history turns up
roughly twenty separate flash/flicker/strobe incidents, each fixed
independently, each with its own spec or retro:

- `#774` / `SPEC_TAB_CONTENT_REVEAL_GATE.md` — tab-switch mount cascade → **reveal gate**
- `SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` — "negative-of-a-photo" first-paint flash → **opacity fade on gate lift**
- `#1769` — browser-pane black flash on macOS/Linux
- `#1809` — activity-summary label flash
- `#2095` — cross-tab drag hover strobe
- `#2098` — flyout menus over browser panes (macOS occlusion)
- `#2151` — Linux: gate window-show/splash-dismiss on real first paint
- `#2163` → **reverted** by `#2169` — CEF New Window startup color flash
- `#2171` / `#2173` / `#2174` — Windows console-window flashes
- `#2293` — activity-dock landing/departure flash
- `#2328` — My Agents loading state during retry
- `#2370` — pin scroll from content ResizeObserver
- `#2525` / `#2567` — submenu positioning flash
- `#2629` — Agent Picker loading flicker → **another combined reveal gate**
- `#2642` — browser-pane loading-brain flicker on load-state flips
- `SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md` — CEF frame-blind `OnLoadingStateChange`
- `SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md` — block-stack mount → **leaf-scoped reveal gate**
- `#2761` — generalize the reveal gate to leaf/pane scope
- `retro-activity-dock-flicker-survives-debounce-fix-2026-08-24.md` — *a debounce fix that did not fix it*
- `#2770` — activity dock flashing stale shell rows
- `SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md` §§2-3, 5, 6, 7 — this one

A full sweep of `docs/specs`, `docs/retro`, `docs/reports` and `docs/status`
puts the real number closer to **forty documents across eleven families** —
tab strip, pane/block mount, activity dock, browser-pane indicators,
startup/splash, terminal streaming, menus/popovers, modals,
transparency/compositor, scroll-pin, and working-row animation.

Four things stand out.

**First, the recurring fix vocabulary is always a *suppressor*, never a
*structure*:** reveal gate, second reveal gate, leaf-scoped reveal gate, opacity
fade, debounce, throttle, settle detector, hard-cap timer, `batch()`, reorder
the emissions. Every one of these hides or delays a symptom on one path. None
of them makes an incorrect intermediate state *impossible to express*.

**Second, at least three of these were themselves follow-ups to a fix that
"should have worked"** — the 08-24 activity-dock retro is literally titled
*"flicker survives debounce fix"*, `#2169` reverted `#2163`, and this spec ran
to a fifth attempt. That is the signature of suppressing a symptom whose cause
lives somewhere the suppressor can't reach.

**Third, the same three anti-patterns recur across unrelated families**, each
already named in the repo's own docs:

- *Whitelist instead of observe.* `REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30.md`
  documents **three prior passes** that "each added one more named dependency
  or observer rather than a structural fix," while the real signal — the
  content box growing — was never observed at all.
- *Coalesce volume, not settlement.* `SPEC_ACTIVITY_DOCK_REFRESH_COALESCING_2026_08_23.md`
  debounced the request storm; one day later the retro records the flicker
  survived, because the intermediate states were **genuinely different true
  backend states**, not redundant repaints. Fewer repaints of a wrong frame is
  still a wrong frame.
- *Gate granularity always trails the defect.* The gate was per-tab (`#774`);
  the flicker reappeared per-leaf (`#2761`); the per-leaf fix then introduced a
  stuck-spinner regression (retro 08-23) **and** a new flicker on the
  previously-clean close path.

**Fourth — and most relevant to §3.3 — the repo has already concluded twice, in
writing, that native-layer gaps cannot be masked by DOM-layer guards.**
`REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md` §4.3 says the DOM
`visibility:hidden` guard "cannot mask any of it" for a non-atomic native
show, and `SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md` Cause 2
finds that a `data-pane-overlay` element's mount/unmount round-trips through
the Windows airspace-clip mechanism (`SetWindowRgn`, `clip.rs`) and performs
"a hard non-animated full native-HWND hide, not a DOM redraw." That is the same
machinery `usePaneOverlay` puts every modal through.

## 3. The actual architecture (as built)

Reading the tab/window render path end to end, here is what is actually there.

### 3.1 State is a bag of independent reactive cells with no transaction boundary

`frontend/app/store/wos.ts` is a `Map<oref, WaveObjectValue>` where **each
object owns its own private Solid signal** (`wos.ts:149`, `createSignal` per
object at `wos.ts:153`). There is no notion of a consistent snapshot across
objects. "One tab closed" is inherently a multi-object fact:

- `Workspace.tabids` — the list (drives *both* the tab strip and the content stack)
- `Workspace.activetabid` — which one is live
- `Tab` — the tab object itself
- `LayoutState` — its pane tree, a *separate* object again
- `Block` objects — one per pane, separate again
- `layoutModelMap` (`frontend/layout/lib/layoutModelHooks.ts:13`) — a **plain non-reactive `Map`** holding the live layout model per tab

Six places, no shared version, no epoch, no transaction. The version guard in
`updateWaveObject` (`wos.ts:280`) is **per object** — it can't detect that
object A is from a newer transition than object B. Any multi-object change is
therefore, structurally, a sequence of independently-observable intermediate
states. `batch()` narrows the window in one delivery path; it does not make the
intermediate states unrepresentable.

### 3.2 There are three transport paths to that state, mutually unordered

For one `CloseTab` call the same `[delete tab, update workspace]` pair arrives:

1. **Wave-obj bridge** (`agentmux-srv/src/server/wave_obj_bridge.rs`) — fires
   off the reducer event stream, *while the HTTP handler is still running*.
   Two separate frames (the second needs an async SQLite fetch), so they can't
   be batched into one.
2. **Response-broadcast loop** (`run_service_call`,
   `agentmux-srv/src/server/service/mod.rs`) — after the handler returns,
   before the response body reaches the caller. Reaches the calling renderer
   too, despite the code comment claiming it's "for everybody else."
3. **HTTP response body** — `respData.updates` in `callBackendService`
   (`wos.ts:137`). Arrives last; by then paths 1-2 have already applied and
   the per-object version guard turns it into a no-op.

§6 batched path 3. §7 batched path 2 and re-ordered path 1. **But the
architecture still has three paths**, and nothing enforces a global ordering or
a shared transaction id between them. The next multi-object gesture that
happens to interleave differently produces a new flash with a new "root cause."

### 3.3 A fourth pipeline that no amount of `batch()` can reach

This is the part I think has been missed in all four previous passes, and it
matches the reported trigger — *"after the modal closes"* — exactly.

Panes are **native surfaces composited by the Rust/CEF host**, not just DOM. To
render a modal above a pane, the frontend punches a hole in the native overlay:
`usePaneOverlay` (`frontend/app/platform/pane-overlay.ts:265`) registers the
modal's rect and calls `sendClip()`. And `sendClip` (`pane-overlay.ts:83`):

```ts
requestAnimationFrame(() => {
    clipScheduled = false;
    flushChain = flushChain.then(() => flushClip()).catch(() => {});
});
```

`flushClip()` then does an **async HTTP round-trip to the Rust host** to update
the native clip.

So when the confirm modal closes:

- **DOM pipeline:** the modal unmounts synchronously, this frame.
- **Native pipeline:** clip removal is deferred to a rAF, then chained onto a
  promise queue, then an async HTTP call, then a native re-composite —
  **several frames later, on a different compositor.**

There is no shared frame boundary and no acknowledgement handshake between the
two. For that interval the native pane layer and the DOM disagree about what
should be on screen. That is a visible flash that is *structurally invisible*
to every fix attempted so far, because every one of them operated inside the
Solid/WOS pipeline.

**Status: real seam, but NOT the cause of the tab-close flash** (see §0). PR
#2818 fixed that symptom without touching this pipeline, so whatever the modal's
clip release costs, it was not what the owner was seeing.

It stays in this report because the seam itself is real and **already
documented twice as a confirmed root cause elsewhere** (§2, fourth point): the
same `data-pane-overlay` → `SetWindowRgn` path produced a hard native-HWND hide
in the browser-pane indicator bug, and the non-atomic native show produced the
new-window color flash. Expect it to surface again in an overlay-heavy path. It
should be ranked on evidence next time, not on narrative fit.

### 3.4 All tabs are mounted at all times, most of them zero-sized

`frontend/app/workspace/workspace.tsx:35-40` keeps **every** tab's content
mounted simultaneously, switching with `display: none`, to preserve xterm.js
scrollback. Consequences:

- A hidden tab's container measures **0×0**. Its `TileLayout` geometry is
  computed against that.
- `useTileLayout` (`layoutModelHooks.ts:69`) puts a **ResizeObserver** on the
  container → `onContainerResize` → `model.updateTree()`
  (`layoutResize.ts:216`). So the instant a tab becomes visible, the observer
  fires 0×0 → real-size and **relayouts the entire pane tree**.
- The codebase already carries scar tissue for exactly this: `layoutModel.ts:443`
  hard-codes a `Math.max(100000, …)` floor because a zero-size measurement
  once parked an overlay on top of a live tab ("the dead source tab of the
  2026-07-11 field sessions"), and `droppable-tab.tsx:246` uses a **double
  `requestAnimationFrame`** to re-measure after a display flip.

So every tab activation is, by construction, a *measure-wrong-then-correct*
sequence. The reveal gate exists to hide that sequence — which brings us to the
last structural problem.

### 3.5 The reveal gate is keyed on the very signal it is trying to gate

`workspace.tsx:65-69`:

```ts
visibility: tid === tabId() && tabSwitching() ? "hidden" : null,
```

The gate applies to whichever tab is **currently active**. But `activeTabId` is
derived from `Workspace.activetabid` (`window-identity.ts:44-49`) — the same
object whose change *is* the transition. So during a close:

- `holdRevealGate()` is called first — at that moment `tabId()` is still the
  **outgoing** tab, so the gate hides *the tab that is about to be deleted*,
  which needs no hiding at all.
- Only once the workspace update lands does `tabId()` flip, and only then does
  the gate cover the **incoming** tab.

The gate is one reactive step behind the thing it protects, and it is a
*time-based* suppressor (`SETTLE_MS = 80`, `MAX_GATE_MS = 800`,
`tab-reveal.ts:51-62`) with no causal link to "the new tab has actually
finished measuring." It guesses. Sometimes it guesses right.

## 4. Diagnosis

The flash is not a bug in the tab-close path. It is the visible symptom of four
structural properties, any one of which is sufficient to produce it:

1. **No transactional boundary over state.** Multi-object truths are stored as
   independent reactive cells, so incoherent intermediate states are always
   representable.
2. **No ordering contract across transports.** Three paths deliver the same
   change; whichever wins the race defines what the user sees.
3. **No cross-compositor synchronization.** DOM and native pane surfaces are
   updated by separate, asynchronous, unacknowledged channels.
4. **Correctness of the visible frame is enforced by timers, not by
   construction.** Reveal gates, debounces and settle detectors *hide* wrong
   frames for a guessed duration instead of preventing them.

Under these properties, "fix the flash" is not a finite task. Each fix removes
one member of a combinatorial set of (transport × object-pair × compositor)
interleavings. That is why four correct fixes moved nothing, and why the
codebase has ~20 of these incidents rather than one.

## 5. What this argues for

Not another gate. The companion spec
(`SPEC_TAB_WINDOW_RENDER_ARCHITECTURE_2026_08_31.md`) proposes the structural
changes; in one line each:

- **A workspace-transition epoch** so multi-object changes are versioned and
  applied as one unit, making torn state unrepresentable rather than merely
  unlikely.
- **One transport for object updates**, with the other two demoted to
  cache-warmers that can never drive a paint.
- **Causal reveal** — reveal on an observed "measured and stable" signal from
  the layout model, replacing the 80ms/800ms timers.
- **A frame contract with the native compositor** so DOM and native clip
  changes commit together, or the DOM waits.

## 6. Honest limits of this analysis

- Written from code and document history. I never visually reproduced the
  flash myself; §0's outcome comes from the fix that shipped.
- §3.3 was my leading hypothesis and was **wrong for this symptom** (§0). The
  mechanism is real and documented elsewhere; my ranking of it was not.
- §3.4 (zero-size measurement) is confirmed by code reading, but its
  contribution to *this* flash remains unquantified — PR #2818 did not address
  it and the symptom went away, so it was likely not a contributor here.
- §3.1/§3.2 (no transaction boundary, three racing transports) remain true of
  the system as built. PR #2818 made the *tab strip* immune to them by not
  depending on backend ordering; it did not remove them. Any other surface that
  still renders straight from multi-object backend state retains the exposure.

## 7. The transferable lesson

The fix that worked did not win the race — it **left the race**. The strip
stopped deriving its rendering from a backend transition it could not order.

That generalizes into a design rule worth applying before the next flicker
bug is filed:

> When a user gesture has a known, predictable outcome, render the outcome
> immediately and reconcile afterwards. Do not render the *transition*, and do
> not try to make the transition atomic — make it invisible.

And a review heuristic, since this class has now cost ~40 documents:

> If a proposed fix is a gate, a debounce, a `batch()`, a delay constant, or an
> emission reorder, it is a **suppressor**. Suppressors are legitimate only
> when the intermediate state is genuinely unavoidable. First ask: can this
> surface stop depending on the ordering altogether?
