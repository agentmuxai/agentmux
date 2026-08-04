// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Crash-time render trail.
//
// SolidJS reconciler crashes (e.g. `replaceChild` NotFoundError) throw
// from deep inside `web.js` with NO user-land frames in the stack —
// `error.stack` is entirely Solid internals, and the effect that
// scheduled the bad DOM op has already returned by the time the catch
// fires. The classic "the call site is gone."
//
// This buffer captures a short-lived ring of recent reactive activity
// (reducer actions, render-effect entries, signal writes the
// instrumentation cares about) so that when `BlockErrorBoundary`
// catches, it can dump "what was happening just before the throw."
// That data does NOT identify the throwing effect directly, but it
// narrows the trigger to whatever reactive write was last in flight.
//
// Design:
//   - Fixed-size in-memory ring (50 entries default).
//   - Zero allocations on hot paths — entries reuse slot objects.
//   - No persistence, no IPC; the boundary dumps the snapshot via the
//     existing `fe_log_structured` channel.
//   - Tag every call site with a short, stable label so log readers
//     can grep ("agent:reducer:TurnStart", "agent:send:start", etc.).

const TRAIL_SIZE = 50;

interface TrailEntry {
    at: number;
    label: string;
    extra?: unknown;
}

const buffer: (TrailEntry | undefined)[] = new Array(TRAIL_SIZE);
let cursor = 0;
let filled = 0;

/**
 * Append a tagged entry to the trail. `extra` is optional and should
 * stay small — it gets JSON-stringified into the host log when the
 * boundary catches. Don't pass DOM nodes or signals.
 */
export function trail(label: string, extra?: unknown): void {
    buffer[cursor] = { at: Date.now(), label, extra };
    cursor = (cursor + 1) % TRAIL_SIZE;
    if (filled < TRAIL_SIZE) filled++;
}

/**
 * Snapshot the trail in chronological order (oldest first). Safe to
 * call from the error boundary — does not mutate the buffer.
 */
export function getTrail(): TrailEntry[] {
    if (filled === 0) return [];
    const out: TrailEntry[] = [];
    // When the buffer is full, oldest is at `cursor`. When not full,
    // oldest is at 0.
    const start = filled < TRAIL_SIZE ? 0 : cursor;
    for (let i = 0; i < filled; i++) {
        const entry = buffer[(start + i) % TRAIL_SIZE];
        if (entry) out.push(entry);
    }
    return out;
}
