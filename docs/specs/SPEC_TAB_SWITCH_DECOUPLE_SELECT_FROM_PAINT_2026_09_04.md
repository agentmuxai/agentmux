# Instant tab-bar selection, decoupled from destination-pane reveal cost (window-level tabs)

**Status:** implemented
**Author:** Posa
**Date:** 2026-09-04
**Scope:** `frontend/app/tab/tabbar.tsx` (`handleSelect`, `displayActiveTabId`), `frontend/app/store/tab-actions.ts`
(`setActiveTab`), `frontend/app/store/window-identity.ts` (`activeTabId`), `frontend/app/workspace/workspace.tsx`
(the tab-content `<For>`'s `display`/`visibility` toggle), `frontend/app/store/tab-reveal.ts`
**Related:** `SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` (profiled the reveal cascade this
spec's fix works around — read first, it's the source of the cost, not something this spec
re-litigates), `SPEC_TAB_CONTENT_REVEAL_GATE.md` (the existing whole-tab visibility gate —
hides the cascade's ugliness but, as explained below, does not and cannot fix the symptom
this spec targets), `SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md` (introduced the exact
optimistic-override pattern this spec proposes extending to a second call site)

---

## 1. The report

> When switching between (window-level) tabs, there is a noticeable delay if the destination
> tab is large — it takes longer for the tab to be *selected*. That's understandable if the
> pane genuinely needs time to load, but that loading time should be cordoned off, visually
> separate from the switch itself. What we have now is a laggard: the tab-bar highlight
> itself lags, and then the complex pane paints immediately right after. We want the
> opposite — the bar switches immediately; the pane takes its time to paint, visibly set
> apart from the switch.

This is not a request to make large panes render faster — that's the pre-existing, distinct
problem `SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` already investigates (and that spec's
own remediation phases are the right place for actually shrinking the reveal cascade's cost).
This is a request to stop that cascade from gating the tab bar's own selection feedback,
which today it does even though the two look, at the code level, like independent signals.

## 2. What already happens today

Every window-level tab stays mounted for the whole session — `workspace.tsx`'s own comment:
"Inactive tabs are hidden via `display:none` — no unmount/remount," backed by a `<For
each={allTabIds()}>` that renders `<TabContent tabId={tid}>` for every tab up front, not just
the active one. So a tab switch is never a remount; it is a pure CSS reveal:
`display: tid === tabId() ? "flex" : "none"`, `tabId()` being `atoms.activeTabId`.

`setActiveTab` (`tab-actions.ts:66-` ) already tries to hide the resulting mess:
`holdRevealGate(tabId)` fires *before* the RPC even starts, applying `visibility: hidden` +
`opacity: 0` to the destination only (targeted per `SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH
_2026_08_25.md` §9, so the *source* tab keeps painting through the round trip instead of the
whole content region blanking). `scheduleRevealLift()` runs in `finally`, starting
`scheduleOnSettle`'s Long-Task-quiet detector (`tab-reveal.ts`, `settle-detector.ts`) so the
gate lifts once the reveal cascade has actually finished, not on a guessed timeout.

This machinery is real and it works — for the destination pane's own appearance. It does
nothing for the tab bar's pill, which is the actual complaint.

## 3. Root cause: one signal, two consumers, no paint boundary between them

`activeTabId` (`window-identity.ts:44`, a `createMemo` over `workspace()`) is
**backend-authoritative**: it only changes once `WorkspaceService.SetActiveTab`'s RPC
round-trips and the resulting `Workspace` object push is processed client-side. Two
completely different pieces of UI read this *same* memo:

1. `tabbar.tsx`'s pill: `isActive={tabId === displayActiveTabId()}` — `displayActiveTabId()`
   (`tabbar.tsx:83-92`) falls straight through to raw `activeTabId()` for an ordinary click
   (its only override today is for the close flow — see §5).
2. `workspace.tsx`'s per-tab `display`/`gateHides` style object, which flips the destination
   from `display:none` to `display:flex` the moment `tabId() === activeTabId()` becomes true
   for it.

Both are driven off the exact same signal write, in the exact same reactive flush, with no
yield point between them. The instant `activeTabId()` updates, Solid synchronously re-runs
every dependent computation before returning control to the browser — including
`workspace.tsx`'s `display:none → flex` flip for the destination, which is precisely the
trigger `SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` identifies as the start of the
expensive cascade: TanStack Virtual's `measureElement`/`getBoundingClientRect()` calls
returning real numbers for the first time (they were 0/empty under `display:none`), markdown
re-render, ResizeObserver/IntersectionObserver fan-out — all synchronous, all scaling with
how much content the destination tab holds.

A browser does not paint mid-task. It paints once the current task, its microtasks, *and*
any ResizeObserver callback batch scheduled for that frame have all drained — and a burst of
newly-triggered ResizeObserver notifications from a large reveal runs exactly in that window,
before paint, in the same frame. `visibility: hidden` keeps the *user* from seeing the ugly
partial cascade, but a hidden element still participates in layout and still fires those
callbacks — so it does nothing to shorten the frame those callbacks are blocking. The tab
bar's pill update sits computed-and-ready in the DOM, but unpainted, until that whole frame's
work — cascade included — finishes. That "unpainted but computed" gap is exactly what reads
as "the tab took a while to be selected."

(`SPEC_TAB_CONTENT_REVEAL_GATE.md`'s original 2026-05-09 description — "Frame 1-2: title bar
/ tabbar updates" ahead of pane content — was accurate to that design's *intent*, written
before `SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` actually profiled how expensive the
reveal cascade could get. The two documents aren't in conflict; the later one is what
revealed that the earlier one's frame-1-2 assumption doesn't hold once the cascade is heavy
enough to consume the same frame the tab bar was hoping to get for free.)

## 4. The fix already exists in this codebase — just not applied here

`tabbar.tsx` already solves exactly this class of problem, for a different trigger.
`pendingHiddenTabIds` / `displayActiveTabId()` (`tabbar.tsx:70-92`,
`SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md`) is an **optimistic, client-only override**
of the backend-authoritative `activeTabId()` for the close flow: the moment a close is
confirmed, the strip immediately shows the neighbor that's *about to* become active, without
waiting for the `CloseTab` RPC to round-trip:

```ts
const displayActiveTabId = () => {
    const real = activeTabId();
    const hidden = pendingHiddenTabIds();
    if (!hidden.has(real)) return real;
    // ...resolve to the neighbor the backend is about to promote...
};
```

`handleSelect` (`tabbar.tsx:94-97`) — the ordinary click-a-tab-to-switch path — has no
equivalent. It calls `setActiveTab(tabId)` directly and the pill waits on the same
`activeTabId()` as everything else, with nothing shielding it from either the RPC latency or
the reveal cascade downstream of the RPC's resolution.

## 5. Proposed design

Extend the same optimistic-override shape `displayActiveTabId` already uses for closes to the
plain-select path:

- Add a `pendingSelectedTabId` signal (or widen `displayActiveTabId`'s existing precedence
  chain to also consult a "user just clicked this" override, alongside its current
  hidden-tab-neighbor-promotion case) — written synchronously in `handleSelect`, **before**
  `setActiveTab(tabId)` is called:

  ```ts
  const handleSelect = (tabId: string) => {
      if (tabId === activeTabId()) return;
      setPendingSelectedTabId(tabId);      // NEW — commits + paints immediately
      setActiveTab(tabId).finally(() => setPendingSelectedTabId(null));
  };
  ```

- `displayActiveTabId()` prefers this pending value over the raw `activeTabId()`, clearing it
  once the real value catches up (or immediately in a `.finally`, since by then the real
  value should already match — a stale mismatch after resolution is a bug to catch in
  testing, not something to paper over).
- **`workspace.tsx`'s own `display`/`gateHides` logic is untouched.** It keeps reading the
  raw, backend-authoritative `atoms.activeTabId` directly, unchanged, still behind the RPC's
  natural async boundary, still protected by the existing visibility/settle gate. Only the
  tab bar's own pill rendering switches to the optimistic value. This mirrors the close flow's
  own division of labor exactly: `displayActiveTabId` is local to `tabbar.tsx` and affects
  only the strip; the content-reveal path in `workspace.tsx` has never depended on it. Nothing
  here is a new architecture — it's the same pattern, applied to the one call site that never
  got it.
- **Failure path:** if `WorkspaceService.SetActiveTab` throws, the `.finally` above still
  clears `pendingSelectedTabId`, so the bar snaps back to the true active tab rather than
  showing a phantom selection forever — mirrors `handleClose`'s own `unhideTab` call in its
  `finally` for the identical class of failure.

## 6. Why this is sufficient without touching the reveal cascade itself

The cascade's cost is real, content-proportional, and this spec does not make it faster —
that stays `SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md`'s job, and its own remediation
phases are the right place to pursue that separately. This fix only stops the cascade's cost
from gating the one thing the user needs immediate confirmation of: that their click landed
and the right tab is now selected. Once decoupled, the destination pane is free to take
however long it needs — already visually cordoned off by the existing `visibility: hidden`
gate — while the bar itself commits and paints on its own, cheap, essentially-instant
schedule.

## 7. Open questions for implementation

- Should the new override generalize into `displayActiveTabId`'s existing precedence chain,
  or stay a second, independently-cleared signal ORed in? `tab-reveal.ts`'s own header
  comment already argues for keeping conceptually distinct gates un-unified ("deliberately
  NOT unified into one data structure... so the already-shipped... gate's behavior and tests
  are untouched") — the same reasoning likely applies here; lean toward two independent
  signals unless implementation finds a concrete reason to merge them.
- Is the RPC round-trip's own latency (before any reveal-cascade cost) actually negligible
  regardless of tab size, or does the pushed `Workspace` update's payload also scale with tab
  count/content (full object vs. diff)? The backend's `set_active_tab`
  (`agentmux-srv/src/backend/wcore/tab.rs:120-133`) is a single-field mutation + `store.update`
  — not proportional to tab content on the Rust side — but this spec has not profiled the
  serialized push payload itself, and that's worth a quick measurement before assuming the
  optimistic signal alone closes the entire gap the report describes.

## 8. Testing

Per this repo's mutation-check discipline: a new test asserting "the pill updates before the
`SetActiveTab` RPC resolves" must be shown to actually fail against today's code (assert
`isActive`/`displayActiveTabId()` flips synchronously on click, before an unresolved RPC
promise settles — e.g. a controllable/deferred mock of `WorkspaceService.SetActiveTab`) before
landing the change that makes it pass.

---

## 9. What shipped

Implemented as described in §5, with the decision extracted into a pure module
rather than left inline:

- **`frontend/app/tab/active-tab-display.ts`** (new) — `resolveDisplayActiveTabId()`,
  the full precedence: optimistic select first (guarded on the tab still existing and
  not itself mid-close), then the pre-existing close-flow neighbor promotion, verbatim.
  Extracted for the same reason `view/agent/failure/synthetic-row.ts` was: the
  precedence between two optimistic overrides is worth asserting directly, and it was
  not reachable while inline in a component.
- **`frontend/app/tab/active-tab-display.test.ts`** (new) — 10 tests. Mutation-checked:
  with the optimistic-select branch removed, exactly the 2 fix-specific tests fail while
  all 8 close-flow tests still pass, confirming the latter guard preserved behavior
  rather than the new behavior.
- **`frontend/app/tab/tabbar.tsx`** — `pendingSelectedTabId` signal; `handleSelect`
  writes it before issuing the RPC and clears it in a `finally`, guarded on the pending
  value still being this call's own target so a newer click during an in-flight RPC is
  not clobbered. `displayActiveTabId` now delegates to the pure function.

`workspace.tsx` was deliberately **not** touched: the destination's `display`/reveal gate
keeps reading the raw backend-authoritative `atoms.activeTabId`, so the content reveal
still happens only on confirmed backend state. Only the strip's highlight is optimistic.

**Verified:** `npx tsc --noEmit` clean for both touched files; 52/52 tests pass across
all four `frontend/app/tab/` suites (the 42 pre-existing plus the 10 new).

**Not measured:** the actual perceived latency improvement in a running build. The
mechanism is verified by construction (the pill no longer reads a value that requires the
RPC, so it cannot be gated on it) and the cost it sidesteps is the 500-600ms browser-side
layout+paint `SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` §"Phase 1 (revised)" already
measured — but a before/after profile on a large tab has not been run.

## 10. Follow-up: honouring the latest click (Codex P2 on PR #2993)

The first cut shipped one `setActiveTab` per click and guarded on the committed
id. Review caught that this loses the newer of two rapid clicks, via **two**
guards, not one:

1. `handleSelect`'s own `if (tabId === activeTabId()) return`. Once the strip can
   display an optimistic selection, displayed and committed diverge — so with
   committed `A` and a pending switch to `B`, clicking `A` again hit this guard
   and did nothing. Fixed by comparing against `displayActiveTabId()`.
2. `setActiveTab`'s own `if (fromTabId === tabId) return` (`tab-actions.ts`).
   Fixing (1) alone is **not** sufficient: re-issuing for `A` while committed is
   still `A` returns without an RPC, so the in-flight `B` call would still land
   and win. The user's last click loses either way.

So a switch is now a loop (`driveTabSelection`), not a call: after each
`setActive` resolves, re-read the intent, and if a newer click arrived, go
again — by then committed has moved off it, so the next call is a real RPC
rather than an early return. It terminates because each iteration either
returns or observes a new intent, which only a fresh user click produces.

`switchLoopRunning` (a plain `let`, not a signal — nothing renders off it) stops
a second click starting a competing loop; the running one picks up the newer
target itself.

Tests: 6 more, mutation-checked against a naive single-call implementation —
exactly the two mid-flight-click cases fail there, including Codex's reported
scenario verbatim, while the other 14 pass.
