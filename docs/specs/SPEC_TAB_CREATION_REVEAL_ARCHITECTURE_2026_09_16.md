# SPEC: Window-Tab Creation Flashing — Root Cause and an Architecture Cleanup

**Date:** 2026-09-16
**Status:** implemented — §3's design shipped as written (`createTab`
creates inactive, populates, then activates via the unmodified
`setActiveTab`; the untargeted gate form retired). Typechecks clean, all
21 existing `tab-reveal.test.ts` cases pass (updated for the retired
untargeted-hold case). Visual verification confirmed a clear improvement;
a residual flash reported specifically on tab CLOSE is tracked separately
(see `SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md` §10 for the
close-specific timing fix shipped alongside this, and the still-open
pane-content flash noted there as unresolved).
**Scope:** window-level tabs only (`frontend/app/workspace/workspace.tsx`,
`frontend/app/store/tab-actions.ts`, `frontend/app/store/tab-reveal.ts`,
`frontend/app/tab/tab-presets.ts`) — specifically the "+" new-tab creation
path. Does **not** touch ordinary tab-*switching* (already-existing tabs)
or in-pane tab switching (a separately-tracked surface, see
`ANALYSIS_CHROME_PARITY_TAB_SWITCH_LATENCY_2026_09_15.md` §6).
**Trigger:** user report, verbatim: *"we are getting weird operation, like
when creating a new tab... we see a lot of flashing. these are window
tabs. we may need an architecture rethink and cleanup."*
**Read first:** `ANALYSIS_CHROME_PARITY_TAB_SWITCH_LATENCY_2026_09_15.md`
(the perf investigation behind PR #3239, which this spec's root cause is a
direct side effect of) and `frontend/app/workspace/workspace.tsx`'s own
extensive inline comments — this is the file whose accumulated patches
this spec is untangling.

---

## 1. The confirmed root cause

`createTab()` (`frontend/app/store/tab-actions.ts:20-55`) holds the
reveal gate through **two** sequential async steps on purpose — the
`CreateTab` RPC, then `applyTabPreset()` (creates the tab's default
agent/sysinfo/swarm panes) — and only calls `scheduleRevealLift()` once
**both** are done, specifically so the new tab doesn't reveal with panes
popping in one at a time. That intent is correct and well-documented
(lines 24-32).

It's undermined by two facts, both verified directly against the code
(not inferred):

1. **The backend activates the new tab as part of the `CreateTab` RPC
   itself**, before any panes exist. `createTab()` calls
   `WorkspaceService.CreateTab(ws.oid, "", true, false)` — the third
   argument is `activate: true`. Server-side
   (`agentmux-srv/src/server/service/tab_lifecycle.rs:98-131`), when
   `activate` is true and the reducer didn't already auto-activate the
   tab (the "first tab in an empty workspace" case), the handler
   dispatches a **separate `Command::SetActiveTab`** to the reducer,
   right there, before the RPC even returns to the client. So
   `ws.activetabid` flips to the new tab's id, and the frontend's
   `tabId()` signal (`frontend/app/workspace/workspace.tsx:18`) updates
   to match, **the moment `CreateTab` resolves** — well before
   `applyTabPreset` even starts.

2. **`applyTabPreset` is guaranteed to take longer than the reveal
   gate's 80ms settle window, entirely in `await`s with no long tasks.**
   `waitForLayoutModel()` (`frontend/app/tab/tab-presets.ts:91-102`)
   opens with `await new Promise((r) => setTimeout(r, 30))` in a polling
   loop, by design ("poll briefly for the layout model to be ready
   rather than racing against the [object] queue") — then walks the
   preset tree issuing a `CreateBlock` RPC per pane (3 for the default
   preset: agent, sysinfo, swarm). None of this registers as a
   PerformanceObserver longtask; it's exactly the kind of "quiet" the
   settle detector (`tab-reveal.ts`'s `startDetector`) is built to treat
   as "done."

3. **`workspace.tsx`'s own tab-switch effect (added in #3239) starts the
   real settle detector the instant it sees `tabSwitching()` is true**,
   independent of which caller is holding the gate or why
   (`workspace.tsx:67-80`):

   ```tsx
   const apply = () => {
       setDisplayTabId(next);
       if (tabSwitching()) scheduleRevealLift();
   };
   ```

   This was added to fix a real, different gap — re-syncing the
   settle-detector's clock to the true `content-visibility` flip moment
   for an ordinary tab *switch*, where `setActiveTab()`'s own
   `finally`-block `scheduleRevealLift()` call already fires right after
   its single RPC resolves (see the file's own comment, lines 47-66).
   For that flow, `tabSwitching()` being true already implies "a
   settle-detector is either running or about to be" — the resync is
   safe and correct.

   For `createTab()`'s flow, `tabSwitching()` is *also* true the whole
   time (set by its own `holdRevealGate()` call before the RPC even
   starts) — but no detector has started yet; `createTab()` deliberately
   hasn't called `scheduleRevealLift()` because `applyTabPreset` isn't
   done. `workspace.tsx`'s effect can't tell these two states apart, so
   it starts the detector anyway, the instant `displayTabId` catches up
   to the newly-activated (but not yet populated) tab — which, per (2),
   is guaranteed to find 80ms of "quiet" while `applyTabPreset` is still
   mid-flight.

**Net effect:** the new tab reveals near-empty, panes pop in one at a
time as each `CreateBlock` RPC resolves, and when `createTab()`'s own,
correctly-timed `scheduleRevealLift()` finally fires afterward (its
`finally` block, once `applyTabPreset` truly completes), it can
re-trigger `setTabSwitching(true)` on an *already-revealed* tab — a
visible reveal → hide → reveal flash. That is the reported "weird
operation... a lot of flashing."

### 1.1 Two secondary, plausible-but-unverified contributors

Not confirmed to the same standard as §1 above — flagged so they're
checked empirically (live `task dev`, console/perf-log verification)
rather than assumed, matching this codebase's own repeated lesson that
this exact file's bugs have gone unnoticed for revisions at a time
without real verification:

- **View Transition into not-yet-rendered content.** A brand-new tab's
  `content-visibility` starts at `"hidden"` the instant it mounts (before
  `displayTabId` ever points to it) — meaning it has **never painted
  once**. Going `hidden → visible` only reuses a cache if one already
  exists; for a tab that's never painted, the "reuse" is a no-op and the
  browser does a full layout+paint anyway. If that first real
  layout+paint hasn't finished by the time `document.startViewTransition`
  takes its "new" snapshot (`workspace.tsx:75-79`), the transition may
  cross-fade from the old tab's full content into a blank/near-blank
  frame, independent of and additional to the gate race in §1.
- **Ref-population timing for the forced-layout effect.**
  `workspace.tsx:112-116`'s `getBoundingClientRect()` fix depends on
  `tabEls.get(id)` already being populated for the destination tab. For
  a brand-new tab, that `ref` callback fires when the `<For>` mounts the
  new div — a separate reactive computation from the sibling
  `createEffect` that reads `tabEls`. Whether the ref is guaranteed to
  have run before this effect reads it, specifically on a tab's very
  first activation, hasn't been verified against Solid's actual
  scheduling (as opposed to assumed from adjacent code).

§2's fix eliminates the *precondition* both of these need (a tab
becoming the display target before its content exists) — see §2.3 — so
neither needs its own patch to be confirmed fixed as a side effect, but
both are worth a live check before closing this out.

---

## 2. Why this keeps happening: the actual design gap

Every fix this subsystem has gotten (targeted vs. untargeted gate holds,
the View-Transition resync, the forced-`getBoundingClientRect` layout
fix) has been a correct, narrow patch for *tab switching* — moving
between two tabs whose content **already exists**, where "is it done
yet" is genuinely unknowable except by watching the browser go quiet.
That's a legitimately heuristic problem, and the settle-detector is a
reasonable tool for it.

Tab *creation* is not that problem. It has an **objective completion
signal** — `applyTabPreset()`'s returned promise — and today it's forced
through the same frame-heuristic gate anyway, via the one caller
(`createTab()`) that holds the gate `untargeted` (`gateTargetTabId ===
null`) specifically because, per the code's own comment
(`tab-reveal.ts:166-167`), *"its destination id doesn't exist until the
RPC returns."* Confirmed via `grep`: `holdRevealGate()`'s untargeted form
has exactly one caller in the whole codebase — `createTab()`. The
untargeted branch of `gateHides()`/`holdRevealGate()` exists **solely**
to serve this one flow.

That's the actual architecture problem: two structurally different
operations — "switch to existing content" (heuristic-timed) and "build
new content, then switch to it" (RPC-timed) — have been threaded through
one shared gate primitive, and the shared primitive's assumptions (§1.3
above) only hold for the first one.

---

## 3. Proposed fix: decouple creation from activation

`agentmux-srv`'s own `CreateTab` handler already treats "create" and
"activate" as two separate reducer commands (§1, point 1) — the frontend
just always fires both together. Stop doing that:

1. `createTab()` calls `WorkspaceService.CreateTab(ws.oid, "", /* activate */
   false, false)`. The new tab exists, but the user's current tab stays
   exactly as it is — active, fully visible, fully interactive. **No
   gate needed for this phase at all**, because nothing the user is
   looking at changes.
2. `applyTabPreset(tabId, DEFAULT_TAB_PRESET)` runs exactly as today,
   populating the new tab's panes — but now entirely in the background.
   The new tab's div is mounted (so the tab-bar pill appears immediately
   — confirms the click registered, standard "new tab" UX — see §3.1)
   but stays `content-visibility: hidden` the whole time, same as any
   other backgrounded tab.
3. Once `applyTabPreset` settles (success **or** failure — a
   partially-populated tab is still better shown than never shown,
   matching that function's own already-documented "return silently on
   any error" philosophy), call the **existing, unmodified**
   `setActiveTab(tabId)`. By this point the destination tab's content is
   already fully built — exactly the scenario `content-visibility` and
   the settle-detector were designed for. The switch that follows is a
   genuine, fast, cached-layout switch, not a race against
   still-in-flight pane creation.

No new signals, no new gate variant — this *removes* code:

- `createTab()` no longer calls `holdRevealGate()` / `scheduleRevealLift()`
  itself at all; `setActiveTab()` already does both, correctly, around
  its own single RPC.
- The untargeted (`null`) branch of `gateTargetTabId` /
  `holdRevealGate()` had exactly one caller (§2) — remove it.
  `holdRevealGate(targetTabId: string)` becomes a required parameter,
  `gateHides()`'s `target == null || target === tid` collapses to
  `target === tid`, and `tab-reveal.ts`'s doc comments documenting the
  "legacy" null case are deleted along with it.

### 3.1 UX consequence, stated explicitly (not hidden in the diff)

Today's (buggy) design *intends* the new tab to become the active tab
immediately, masked behind a reveal gate while it populates. This
proposal changes that intent: the new tab populates in the background
and the switch happens once, atomically, when it's ready — the tab-bar
pill appears right away (unaffected by any of this — the tab bar isn't
part of `workspace.tsx`'s gated content area), but the content area
doesn't switch to it until then. For the default preset (3 panes, a few
fast RPCs) this should be a short, sub-second delay; if `applyTabPreset`
is ever slow, the user sees their current tab stay fully interactive
throughout instead of watching the new one flash together — closer to
how Chrome's own "new tab opens, populates, you land on it" behavior
reads than what either the old (`display:none`) or current
(content-visibility-with-race) mechanism produces.

### 3.2 What does NOT change

- `workspace.tsx`'s `content-visibility` / `pointer-events` / View
  Transition / forced-`getBoundingClientRect` mechanism for ordinary tab
  switching — untouched, presumed correct for its actual use case
  (switching between tabs with existing content).
- `setActiveTab()` itself — untouched, reused as-is.
- Tab close / promote-neighbor (`tabbar.tsx:175`) — already a targeted
  `holdRevealGate(promotedTabId)` call, unaffected.
- In-pane tab switching — out of scope (separate surface, see the Scope
  line above).

---

## 4. Verification plan

Per this codebase's own established rule for this exact subsystem
("don't skip Phase 0... if the measurement doesn't move, the PR doesn't
ship"):

1. **Repro the bug on current `main` first**, to have a real before:
   `muxlog host` while clicking "+" a few times, grep for `[perf]
   long-task` and the reveal-gate's own lift timing between the click
   and the panes finishing — should show the premature lift (gate lifts
   well before the 3rd `CreateBlock` RPC resolves) confirming §1.
2. Implement §3, then repeat the same repro: the gate should lift
   exactly once, after `applyTabPreset` resolves, with no intervening
   reveal.
3. Live `task dev` visual check (screenshot or direct observation) of
   creating a new tab several times in a row, confirming no visible
   flash and that the tab-bar pill appears immediately even before the
   content switch completes.
4. Confirm the two secondary contributors (§1.1) no longer reproduce —
   if either still does, they need their own follow-up, not folded
   silently into this fix's acceptance.
5. Existing test coverage: `tab-reveal.test.ts`, `workspace.tsx`'s own
   tests (if any) — update for the removed untargeted-gate branch;
   `tab-actions.ts` currently has none directly, worth adding one for
   the new create → populate → activate sequencing.
