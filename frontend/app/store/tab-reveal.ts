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
//     per-instance-cancellable primitive for it.
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
 */
export function holdRevealGate(): void {
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

const [gatingNodeIds, setGatingNodeIds] = createSignal<ReadonlySet<string>>(new Set());
export { gatingNodeIds };

/** Cancel function for whichever timer/detector is currently live for a
 *  given node id — either a plain hold-fallback `setTimeout`, or a
 *  `scheduleOnSettle` detector. At most one entry per node id; a new
 *  hold/schedule call for the same id cancels and replaces it. */
const leafCancels = new Map<string, () => void>();

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

/**
 * Pin a single leaf's reveal gate up, by layout node id — the block-stack
 * analog of `holdRevealGate()`. Use before the async work that precedes a
 * `pushBlockOntoStack`/`setActiveBlockInStack` call (the RPCs allocating
 * or resolving the block to attach), so the leaf stays hidden through the
 * whole operation instead of revealing mid-flight.
 *
 * Same pairing contract as `holdRevealGate()`: callers MUST follow up with
 * `scheduleLeafRevealLift(nodeId)` (typically in a `finally`), and get the
 * same MAX_GATE_MS safety-net fallback if they don't.
 */
export function holdLeafRevealGate(nodeId: string): void {
    addGatingNode(nodeId);
    cancelLeaf(nodeId);
    const timer = setTimeout(() => {
        leafCancels.delete(nodeId);
        removeGatingNode(nodeId);
    }, MAX_GATE_MS);
    leafCancels.set(nodeId, () => clearTimeout(timer));
}

/**
 * Mark one leaf's gate active and start watching for clean frames.
 * Idempotent per node id — a second call for the same `nodeId` before the
 * first completes resets that leaf's own detector; it never touches any
 * other leaf's (`scheduleOnSettle` is a fresh, independently-cancellable
 * instance per call).
 */
export function scheduleLeafRevealLift(nodeId: string): void {
    addGatingNode(nodeId);
    cancelLeaf(nodeId);
    const cancel = scheduleOnSettle(
        () => {
            leafCancels.delete(nodeId);
            removeGatingNode(nodeId);
        },
        { settleMs: SETTLE_MS, maxMs: MAX_GATE_MS },
    );
    leafCancels.set(nodeId, cancel);
}
