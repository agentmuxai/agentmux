# Tab content reveal gate

**Status:** Implemented — but this document describes the ORIGINAL design; two
later specs changed its behaviour. Read those before relying on anything below.
**Owner:** AgentA
**Date:** 2026-05-09

> **2026-08-07 audit note:** Implemented — `tab-reveal.ts`/`tab-reveal.test.ts`
> exist, wired into `startup-splash.ts` and `editor-model.ts`. Status field
> was never updated. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.
>
> **2026-08-31 amendment — the gate is no longer one global boolean:**
> - `SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md` added a **leaf-scoped**
>   gate (`gatingNodeIds()`, `holdLeafRevealGate`/`scheduleLeafRevealLift` with
>   generation tokens) alongside this whole-tab one, for pane-local mounts that
>   bypass `setActiveTab`.
> - `SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md` §9 made the whole-tab
>   gate **targeted**: `holdRevealGate(targetTabId)` names its destination, and
>   `workspace.tsx` hides only that tab once it becomes active. Previously it
>   gated whichever tab was active *at the time*, which — because `activeTabId`
>   derives from the same `Workspace` object whose mutation is the transition —
>   meant it gated the OUTGOING tab during a close and let the incoming one
>   blank toward the 800ms cap.
>
> The timer-based lift below (80ms clean-frame settle / 800ms hard cap) is
> unchanged and remains the known weak point — it guesses at readiness rather
> than observing it. See
> `docs/specs/SPEC_TAB_WINDOW_RENDER_ARCHITECTURE_2026_08_31.md` §3.4 for the
> proposed causal replacement.
**Driving observation:** Switching tabs in AgentMux reveals content in stages — title bar updates first, then pane shells, then block content fills in pane-by-pane, then final layout settles. The user perceives this as visual jank: "different parts of the window appear at different times." Reported as a general cleanup concern, not specific to any one pane type.

## Symptom

After clicking a tab (or pressing Ctrl+Tab), the new tab's content appears in 3-5 visible stages:

1. **Frame 1-2** — title bar / tabbar updates to reflect new active tab.
2. **Frame 3-5** — pane frames render (the divider grid is visible but panes are empty).
3. **Frame 6-15** — pane contents stream in. Order is non-deterministic — terminal might paint before agent, or vice versa.
4. **Frame 16-30** — late-arriving content (markdown blocks, code-syntax-highlight, browser HWNDs reattaching) finishes resolving.

Each stage is a separate paint. The eye registers each transition as flicker. By the time everything's stable, the user has been looking at a half-rendered tab for ~150-300ms.

The new tab-switch perf marks (PR #772) capture the *total* duration; this spec addresses the *visual quality* of what happens during that duration.

## Root cause

SolidJS's reactive system mounts components independently as their data resolves. There's no orchestration that says "delay paint until everything is ready." Each pane's content is a separate reactive subgraph: agent panes load from JSONL on disk, browser panes wait for CEF HWND attach, terminal panes wait for blockfile read. Each finishes at its own pace and triggers its own paint.

This is the **right** default behavior for a streaming app — content is shown as it arrives — but it's the **wrong** default for a tab switch where the user wants a clean, atomic "before/after" transition.

## Design space

### Option A — Frame-budget gate (recommended)

After tab switch, hide the new tab's content with a CSS `visibility: hidden` overlay. A frame-budget detector watches for **N consecutive "good" frames** (no Long Tasks > 16ms in a sliding window). Once detected, lift the overlay → content reveals atomically.

**Pros:**
- No per-pane coordination needed. Works for every pane type, present and future.
- Reuses the Long Tasks observer already in `frontend/perf/observers.ts`.
- Self-tuning — a pane that loads fast doesn't pay extra latency; a pane that loads slow naturally takes longer.

**Cons:**
- Adds 100-300ms perceived latency on tab switch (intentionally — that's the cost of stability).
- Requires a fallback timeout to avoid infinite gating if a pane has a permanent low-rate background task.
- The gate's own DOM operations (visibility toggle) trigger their own reflow when lifted — may show a brief blink as the just-revealed content settles.

### Option B — Promise-based readiness gate

Each block type declares a `whenReady` promise. The tab container holds a `Promise.allSettled` over every block's promise (with timeout) and reveals when all resolve.

**Pros:**
- Deterministic — only reveals when content is actually ready.
- Per-block visibility into "what's slow" via promise instrumentation.

**Cons:**
- Heavy coordination cost — every block type (agent, browser, terminal, editor, swarm, sysinfo, memory, help, devtools) needs a `whenReady` contract.
- Some blocks have no clear "ready" — when is a streaming agent ever fully ready? Need block-specific definitions.
- Blocks render via SolidJS createEffect; promise lifecycle doesn't compose cleanly with reactive lifecycle.

### Option C — Skeleton + progressive reveal

Show skeleton placeholders for each block until that block's content is ready, then crossfade. Doesn't gate the whole tab — just makes each transition visually smooth.

**Pros:**
- Keeps app feeling responsive — content streams as before, just with cleaner transitions.
- Matches industry-standard pattern (Slack, Linear, Notion all do skeletons).

**Cons:**
- Requires per-block skeleton design (visual + CSS work).
- Skeletons themselves can be visually noisy if block types differ (a terminal skeleton vs. a code-diff skeleton).
- Doesn't fully solve the symptom — user still sees N animations instead of 1.

### Option D — Fixed-duration CSS hide

`visibility: hidden` for a fixed window (e.g., 200ms) post-switch, lifted unconditionally.

**Pros:**
- Simplest to implement — no observer, no coordination.
- Predictable latency.

**Cons:**
- Wrong duration for any specific case — too long for fast tabs, too short for slow ones.
- No recovery if real latency exceeds the fixed window — user sees half-rendered content anyway.

## Recommendation: **Option A** (frame-budget gate) with **Option D** (CSS visibility) as the mechanism

Use `visibility: hidden` (preserves layout, suppresses paint, cheaper than `display: none`) on the tab container during the gate. Drive the lift via the Long Tasks observer.

## Implementation outline

### Frontend signal

```ts
// frontend/app/store/tab-reveal.ts (new file)
const [tabSwitching, setTabSwitching] = createSignal(false);
```

### Wrap setActiveTab

In `frontend/app/store/global.ts`'s `setActiveTab`:

```ts
export async function setActiveTab(tabId: string): Promise<void> {
    const ws = workspace();
    if (ws == null) return;
    const fromTabId = activeTabId();
    if (fromTabId === tabId) return;

    setTabSwitching(true);
    markStart("tab-switch", { from: fromTabId, to: tabId });
    try {
        await WorkspaceService.SetActiveTab(ws.oid, tabId);
    } finally {
        // Existing markEnd via double-rAF (PR #772).
        requestAnimationFrame(() => requestAnimationFrame(() => markEnd("tab-switch")));
        scheduleRevealLift();
    }
}
```

### Reveal-lift detector

```ts
function scheduleRevealLift() {
    const startedAt = performance.now();
    const MAX_GATE_MS = 800;          // hard cap
    const SETTLE_MS = 80;              // need 80ms of clean frames
    const LONG_TASK_THRESHOLD_MS = 50; // any task longer = "not settled"

    let lastLongTaskAt = startedAt;
    const handler = (entries: PerformanceObserverEntryList) => {
        for (const e of entries.getEntries()) {
            if (e.duration > LONG_TASK_THRESHOLD_MS) {
                lastLongTaskAt = performance.now();
            }
        }
    };
    const observer = new PerformanceObserver(handler);
    observer.observe({ entryTypes: ["longtask"] });

    const tick = () => {
        const now = performance.now();
        if (now - lastLongTaskAt >= SETTLE_MS || now - startedAt >= MAX_GATE_MS) {
            observer.disconnect();
            setTabSwitching(false);
            return;
        }
        requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
}
```

### Tab container CSS

```css
/* on the tab content root */
.tab-content--switching {
    visibility: hidden;
    /* Prevent late paint from flashing if MAX_GATE_MS hits */
}
```

### Apply to tab content

In whatever component renders the active tab's pane tree (likely `frontend/app/workspace/workspace.tsx` or a layout component):

```tsx
<div classList={{ "tab-content--switching": tabSwitching() }}>
  {/* existing tab content */}
</div>
```

## Edge cases

- **Background activity unrelated to tab switch.** A streaming agent in another tab generates Long Tasks. The detector should be **scoped to the current tab's content paint** if possible — but since Long Tasks are global, we can't filter perfectly. Mitigation: the 800ms hard cap. In practice, agent-pane streaming Long Tasks are usually < 50ms each (per current data) so they don't trigger.
- **Rapid sequential switches** (Ctrl+Tab held down). Each switch should reset the detector. The implementation above naturally does this because `setActiveTab` re-fires `setTabSwitching(true)` and `scheduleRevealLift()` for each switch.
- **First-paint after app start.** App start has its own initialization phase with many Long Tasks; we should NOT gate the initial paint. Solution: only call `scheduleRevealLift` from `setActiveTab` (an explicit user action), not from the initial atom hydration.
- **Reduced-motion users.** The settling lift is instantaneous (no animation), so reduced-motion is unaffected. No special handling needed.

## Out of scope

- **Per-block skeletons** (Option C). Could be added later as a complementary visual layer; this spec doesn't require it.
- **`whenReady` block contract** (Option B). Would need a separate spec; useful for telemetry but heavy lift.
- **Fade-in transition** when revealing. CSS opacity transition could be added, but introduces its own paint cost. Start with instantaneous reveal; revisit if jarring.
- **Backend coordination.** Backend-driven switches (tearoff merge, cross-window drag) bypass `setActiveTab` and don't currently fire the gate. They're rare; address with a separate `setTabSwitching(true)` call in those handlers if it becomes a pain point.

## Effort

| Component | LOC | Days |
|---|---|---|
| `tab-reveal.ts` signal + scheduleRevealLift | ~60 | 0.25 |
| `setActiveTab` integration | ~5 | — |
| Tab-container CSS class + JSX wiring | ~20 | 0.25 |
| Test the gate doesn't trigger on background streams | ~30 (observer mock) | 0.25 |
| **Total** | ~115 | **~0.75 day** |

## Cross-references

- Tab-switch mark coverage (PR #772) — measures total switch duration, complements this spec's stability gate.
- Agent pane virtualization (PR #773) — reduces the per-tab Long Tasks that this gate is waiting on.
- Phase 0 perf observers: `frontend/perf/observers.ts` — Long Tasks observer this design reuses.

## Driving observation (verbatim)

> "piece meal is something i have noticed everywhere, it has nothing to do with the agent pane .. its just a general cleanup thing we need."
