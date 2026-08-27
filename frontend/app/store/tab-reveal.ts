// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tab-content reveal gate (issue #774, spec
// `docs/specs/SPEC_TAB_CONTENT_REVEAL_GATE.md`), plus its leaf-scoped
// generalization (`docs/specs/SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md`).
//
// Hides content under `visibility: hidden` while a fresh switch/open/mount
// is settling, then reveals it once a window of "clean" frames (no Long
// Tasks > 50ms) has passed — or the hard cap trips. The user perceives an
// atomic before/after transition instead of a piecemeal mount cascade.
//
// Two independent gates exist here, deliberately NOT unified into one data
// structure keyed by some "whole-tab" sentinel, so the already-shipped
// whole-tab gate's behavior and tests are untouched by the leaf-scoped
// addition:
//
//   - `tabSwitching` / `holdRevealGate()` / `scheduleRevealLift()` — the
//     ORIGINAL whole-tab gate, with its own hand-rolled detector
//     (`startDetector`/`armFallback`/`cancelDetector` below). Triggered
//     from `setActiveTab`/`createTab` in `tab-actions.ts`. Unchanged from
//     before this generalization.
//   - `gatingNodeIds()` / `holdLeafRevealGate(nodeId)` /
//     `scheduleLeafRevealLift(nodeId)` — the NEW leaf-scoped gate. Covers
//     backend-driven, pane-local mounts that bypass `setActiveTab` entirely
//     (block-stack pushes: the "+" new-agent-tab button, Quick Fork, Agent
//     History) — exactly the class `SPEC_TAB_CONTENT_REVEAL_GATE.md`'s own
//     "Out of scope" section flagged as a likely future need. Built on
//     `@/app/util/settle-detector`'s `scheduleOnSettle` rather than
//     duplicating the whole-tab gate's own detector — that module's own
//     doc comment already anticipated exactly this need ("wrong for N
//     agent panes each waiting on their own settle, since concurrent
//     callers would clobber each other's detector"), pre-built as a
//     per-instance-cancellable primitive for it. `clearLeafRevealGate(nodeId)`
//     must be called when a node is permanently removed from the layout
//     tree (wired from `closeNode` in `layoutMagnify.ts`) — otherwise the
//     per-node bookkeeping Maps below grow unboundedly over a long-running
//     session (reagent's review of PR #2761).
//
// Reduced-motion behaviour: the reveal is unanimated regardless, so
// `prefers-reduced-motion` users get the same behaviour. No special
// handling needed.

import { createSignal } from "solid-js";
import { fadeOutStartupSplash } from "@/app/init/startup-splash";
import { scheduleOnSettle } from "@/app/util/settle-detector";

/** Hard cap on how long a gate stays up. Past this, content reveals even
 *  if the long-task stream hasn't gone quiet — protects against perma-busy
 *  content (streaming agent, etc.) holding the gate open. */
const MAX_GATE_MS = 800;

/** A window of clean frames (no long tasks beyond `LONG_TASK_THRESHOLD_MS`)
 *  of at least this duration counts as "settled". 80 ms is ~5 frames at
 *  60 Hz — empirically enough to cover the bulk of post-mount measurement
 *  reflow without being noticeably slow. */
const SETTLE_MS = 80;

/** Any task at least this long is treated as "busy" — the settle clock
 *  restarts when one fires. Matches PerformanceObserver's default
 *  longtask threshold; calling it out so we can tune from one place. */
const LONG_TASK_THRESHOLD_MS = 50;

// ─── Shared detector primitive ──────────────────────────────────────────
//
// One `DetectorHandle` per independently-gated thing (the whole tab, or
// one leaf node). Holds whichever of {observer, fallback timer} is
// currently live for that thing, so a new hold/schedule call can cancel
// exactly its own prior detector without touching any other's.

interface DetectorHandle {
    observer: PerformanceObserver | null;
    fallbackTimer: ReturnType<typeof setTimeout> | null;
}

function newHandle(): DetectorHandle {
    return { observer: null, fallbackTimer: null };
}

function cancelDetector(handle: DetectorHandle): void {
    handle.observer?.disconnect();
    handle.observer = null;
    if (handle.fallbackTimer !== null) {
        clearTimeout(handle.fallbackTimer);
        handle.fallbackTimer = null;
    }
}

function armFallback(handle: DetectorHandle, onSettle: () => void, ms: number): void {
    handle.fallbackTimer = setTimeout(() => {
        handle.fallbackTimer = null;
        onSettle();
    }, ms);
}

/** Start (or restart) the long-task-free settle detector for `handle`,
 *  calling `onSettle` once a clean window is observed or the hard cap
 *  trips. Idempotent re-entry: cancels any detector already running on
 *  this same handle first. */
function startDetector(handle: DetectorHandle, onSettle: () => void): void {
    const startedAt = performance.now();
    let lastLongTaskAt = startedAt;

    cancelDetector(handle);

    if (typeof PerformanceObserver === "undefined") {
        // No PerformanceObserver in this runtime (test env, etc.). Fall
        // back to the hard cap — without longtask data we can't detect
        // the actual settle moment, so wait the full MAX_GATE_MS budget
        // rather than the shorter SETTLE_MS, which would reveal mid-mount.
        armFallback(handle, onSettle, MAX_GATE_MS);
        return;
    }

    let observer: PerformanceObserver;
    try {
        observer = new PerformanceObserver((entries) => {
            for (const e of entries.getEntries()) {
                if (e.duration > LONG_TASK_THRESHOLD_MS) {
                    lastLongTaskAt = performance.now();
                }
            }
        });
        observer.observe({ entryTypes: ["longtask"] });
        handle.observer = observer;
    } catch {
        // longtask observer not supported (Safari historically). Same
        // hard-cap fallback reasoning as the no-PO path above.
        armFallback(handle, onSettle, MAX_GATE_MS);
        return;
    }

    const tick = () => {
        // Identity check against the captured observer — `handle.observer`
        // may now point to a newer observer that a subsequent start call
        // installed. If so, this older tick must NOT fire `onSettle`: the
        // newer detector owns this handle now.
        if (handle.observer !== observer) return;

        const now = performance.now();
        const settledSinceLastBusy = now - lastLongTaskAt >= SETTLE_MS;
        const hardCapHit = now - startedAt >= MAX_GATE_MS;

        if (settledSinceLastBusy || hardCapHit) {
            observer.disconnect();
            handle.observer = null;
            onSettle();
            return;
        }
        requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
}

// ─── Whole-tab gate (original — unchanged behavior) ─────────────────────

const [tabSwitching, setTabSwitching] = createSignal(false);
export { tabSwitching };

// When non-null, the gate hides ONLY this tab id (once it becomes the
// active tab) instead of whichever tab is currently active. Set by
// destination-aware holders (tab close promotion, tab switches): the
// SOURCE tab then stays visible right up to the activetabid flip instead
// of blanking the content region the moment the gate goes up — the
// "neighbor pane flash" of SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH §9. Null
// (legacy) hides the current active tab, which createTab still wants
// (its destination id doesn't exist until the RPC returns).
const [gateTargetTabId, setGateTargetTabId] = createSignal<string | null>(null);
export { gateTargetTabId };

const tabHandle = newHandle();

/** Lift the whole-tab gate AND cross-fade the startup splash. The gate's
 *  "settled" moment is exactly when the window's content is ready to show,
 *  so it's also when the full-cover brain splash should fade out — keeping
 *  it on top of the entire bootstrap until then (see startup-splash.ts).
 *  `fadeOutStartupSplash` is idempotent: a no-op once the splash is gone,
 *  so calling it on every gate lift (including ordinary tab switches) is
 *  safe. */
function liftTabGate(): void {
    setTabSwitching(false);
    setGateTargetTabId(null);
    fadeOutStartupSplash();
}

/**
 * Pin the whole-tab reveal gate up without starting the auto-lift
 * detector. Use before async work (RPC, dynamic import, layout-model
 * polling) where `scheduleRevealLift()` would let the SETTLE window
 * elapse during the await with no longtasks firing, prematurely revealing
 * an empty or half-mounted tab.
 *
 * Callers MUST pair `holdRevealGate()` with a follow-up
 * `scheduleRevealLift()` (typically in a `finally`) once the async work
 * resolves. As a safety net for the case where the awaited promise never
 * settles (e.g. a backend `fetch` that hangs without a timeout —
 * `callBackendService` does not impose one), this also arms a
 * MAX_GATE_MS fallback so the gate eventually lifts and the window can't
 * be left blank indefinitely. The normal-path `scheduleRevealLift()`
 * cancels this timer before installing its own detector.
 *
 * @param targetTabId when the destination tab of the transition is known
 *   up front (tab switch, close-promotion), pass it: the gate then hides
 *   only that tab once it becomes active, and the source keeps painting
 *   until the actual activetabid flip — no premature blank. Omit (null)
 *   for the legacy hide-current-active behavior (createTab, where the
 *   destination id doesn't exist yet).
 */
export function holdRevealGate(targetTabId: string | null = null): void {
    setGateTargetTabId(targetTabId);
    setTabSwitching(true);
    cancelDetector(tabHandle);
    armFallback(tabHandle, liftTabGate, MAX_GATE_MS);
}

/**
 * Mark the whole-tab gate active and start watching for clean frames.
 * Idempotent — a second call before the first completes resets the
 * detector. That's what handles rapid Ctrl-Tab spam.
 */
export function scheduleRevealLift(): void {
    setTabSwitching(true);
    startDetector(tabHandle, liftTabGate);
}

// ─── Leaf-scoped gate (generalization — SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22) ──
//
// Generation-token design (Codex's review of PR #2761 caught two real races
// in an earlier, generation-less version of this gate):
//
//   1. Two overlapping operations on the SAME node id (e.g. two rapid "+"
//      clicks before the first's RPC resolves) — the OLDER operation's
//      `finally`-scheduled lift must not touch the gate the NEWER
//      operation now owns, or the pane reveals mid-second-operation.
//   2. A single SLOW operation whose own `holdLeafRevealGate` safety-net
//      timer fires (revealing the pane) before the operation actually
//      finishes — the later `scheduleLeafRevealLift` call must not re-hide
//      an already-revealed pane, or the user sees a jarring
//      visible→hidden→visible sequence.
//
// Every `holdLeafRevealGate` call mints a fresh generation number for its
// node id and returns it; the paired `scheduleLeafRevealLift` call takes it
// back. A call is a no-op whenever its generation is stale — either a NEWER
// generation has since started (case 1), or its OWN generation already
// resolved once, by timeout or settle (case 2, tracked via
// `leafResolvedGeneration`).

const [gatingNodeIds, setGatingNodeIds] = createSignal<ReadonlySet<string>>(new Set());
export { gatingNodeIds };

/** Cancel function for whichever timer/detector is currently live for a
 *  given node id — either a plain hold-fallback `setTimeout`, or a
 *  `scheduleOnSettle` detector. At most one entry per node id; a new
 *  hold/schedule call for the same id cancels and replaces it. */
const leafCancels = new Map<string, () => void>();

/** The generation number of the MOST RECENT `holdLeafRevealGate` call for
 *  a given node id. */
const leafGeneration = new Map<string, number>();

/** The highest generation number that has ALREADY had its gate lifted
 *  (by settle-detection or its own hold's safety-net timeout) for a given
 *  node id. A `scheduleLeafRevealLift` call for a generation at or below
 *  this is a no-op — re-hiding an already-revealed pane is worse than
 *  leaving it visible while a slow operation finishes. */
const leafResolvedGeneration = new Map<string, number>();

function addGatingNode(nodeId: string): void {
    setGatingNodeIds((prev) => {
        if (prev.has(nodeId)) return prev;
        const next = new Set(prev);
        next.add(nodeId);
        return next;
    });
}

function removeGatingNode(nodeId: string): void {
    setGatingNodeIds((prev) => {
        if (!prev.has(nodeId)) return prev;
        const next = new Set(prev);
        next.delete(nodeId);
        return next;
    });
}

function cancelLeaf(nodeId: string): void {
    leafCancels.get(nodeId)?.();
    leafCancels.delete(nodeId);
}

function currentLeafGeneration(nodeId: string): number {
    return leafGeneration.get(nodeId) ?? 0;
}

/** True if `generation` is no longer the one this node id's gate should
 *  listen to — see the two race cases in the module doc comment above. */
function isLeafGenerationStale(nodeId: string, generation: number): boolean {
    return (
        generation !== currentLeafGeneration(nodeId) ||
        generation <= (leafResolvedGeneration.get(nodeId) ?? 0)
    );
}

/** Common "this generation is done" path for both the hold-timeout and the
 *  settle-detected outcomes. A no-op if a newer generation has since taken
 *  over — that generation owns the gate now and will resolve it itself. */
function resolveLeafGeneration(nodeId: string, generation: number): void {
    leafCancels.delete(nodeId);
    if (generation !== currentLeafGeneration(nodeId)) return;
    leafResolvedGeneration.set(nodeId, generation);
    removeGatingNode(nodeId);
}

/**
 * Pin a single leaf's reveal gate up, by layout node id — the block-stack
 * analog of `holdRevealGate()`. Use before the async work that precedes a
 * `pushBlockOntoStack`/`setActiveBlockInStack` call (the RPCs allocating
 * or resolving the block to attach), so the leaf stays hidden through the
 * whole operation instead of revealing mid-flight.
 *
 * @returns an opaque generation token. Callers MUST pass it to the paired
 *   `scheduleLeafRevealLift(nodeId, generation)` call (typically in a
 *   `finally`) — see the module doc comment above for why a raw
 *   `scheduleLeafRevealLift(nodeId)` with no generation would be unsafe
 *   once two operations can overlap on the same node id. Also gets the
 *   same MAX_GATE_MS safety-net fallback as `holdRevealGate()` if the
 *   paired call never arrives.
 */
export function holdLeafRevealGate(nodeId: string): number {
    addGatingNode(nodeId);
    cancelLeaf(nodeId);
    const generation = currentLeafGeneration(nodeId) + 1;
    leafGeneration.set(nodeId, generation);
    const timer = setTimeout(() => resolveLeafGeneration(nodeId, generation), MAX_GATE_MS);
    leafCancels.set(nodeId, () => clearTimeout(timer));
    return generation;
}

/**
 * Mark one leaf's gate active and start watching for clean frames, for the
 * generation token returned by the paired `holdLeafRevealGate` call. A
 * no-op if that generation is stale (superseded by a newer hold, or
 * already resolved by its own hold's safety-net timeout) — see the module
 * doc comment above.
 */
export function scheduleLeafRevealLift(nodeId: string, generation: number): void {
    if (isLeafGenerationStale(nodeId, generation)) return;
    addGatingNode(nodeId);
    cancelLeaf(nodeId);
    const cancel = scheduleOnSettle(
        () => resolveLeafGeneration(nodeId, generation),
        { settleMs: SETTLE_MS, maxMs: MAX_GATE_MS },
    );
    leafCancels.set(nodeId, cancel);
}

/**
 * Drop all reveal-gate bookkeeping for a layout node id that no longer
 * exists — `leafCancels`/`leafGeneration`/`leafResolvedGeneration` are
 * otherwise only ever added to or overwritten, never deleted, so a
 * long-running session with many pane splits/closes would grow these Maps
 * unboundedly (reagent's review of PR #2761). Call this from wherever a
 * node is permanently removed from the layout tree — `closeNode`
 * (`layoutMagnify.ts`) is the one place every close path (block-stack pop,
 * ordinary pane close, drag tear-off) funnels through.
 *
 * Safe to call for a node that was never gated (all three lookups are
 * no-ops) or is mid-gate (cancels its pending timer/detector first, same
 * as a fresh hold/schedule would, so no dangling callback fires for a
 * node id that's already gone).
 */
export function clearLeafRevealGate(nodeId: string): void {
    cancelLeaf(nodeId);
    leafGeneration.delete(nodeId);
    leafResolvedGeneration.delete(nodeId);
    removeGatingNode(nodeId);
}
