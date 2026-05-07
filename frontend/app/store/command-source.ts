// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Command source-tagging + global event log (slice #3 of the frontend
 * reducer roadmap). See docs/specs/frontend-reducer-implementation-plan-2026-05-03.md
 * PR-C and the conventions doc §10 Q4.
 *
 * The reducer slices each maintain their own state. This module sits
 * orthogonal to them and tracks WHO initiated each command (user,
 * agent, system) plus a single global ring buffer of recent dispatches
 * for diagnostics.
 *
 * Per-slice dispatch functions accept an optional `source` parameter
 * (default `"system"`) and call `recordDispatch(...)` after applying
 * the command. The diagnostics panel will surface `getRecentDispatches`
 * in a follow-up PR.
 *
 * In-memory only — opt-in file persistence is deferred until a
 * concrete debugging need surfaces (per the plan's lean default).
 */

import { createSignal, untrack, type Accessor } from "solid-js";

/**
 * Who initiated a command. Defaults to `"system"` when not specified
 * (background subscriptions, lifecycle callbacks, etc.).
 *
 * - `"system"` — internal flow (stream tick, history load, mirror sub)
 * - `"user"` — direct user action (click, slash command, keystroke)
 * - `{ kind: "agent"; agentId }` — agent app-API surface (future; no
 *   callers yet, but the type is in place for when the surface lands)
 */
export type CommandSource =
    | "system"
    | "user"
    | { kind: "agent"; agentId: string };

/**
 * One entry in the global dispatch log. `command` and `events` are
 * `unknown` because each slice has its own discriminated unions; the
 * log is generic. Diagnostics surface that wants to render structured
 * detail can downcast based on the `slice` discriminator.
 */
export interface DispatchRecord {
    /** Identifier for the slice that handled the command (e.g. "agent-document"). */
    slice: string;
    /** Per-slot key when the slice has slots (e.g. blockId). null for global slices. */
    key: string | null;
    command: unknown;
    events: ReadonlyArray<unknown>;
    source: CommandSource;
    /** ms epoch. */
    at: number;
}

const RING_CAPACITY = 500;

/**
 * Ring buffer of recent dispatches. Solid signal so a diagnostics
 * panel (PR-C follow-up) can re-render reactively. Reads are O(1)
 * (signal getter); writes shift+push at O(n) when full but n is
 * bounded at RING_CAPACITY so amortized cost is fine.
 */
const [recordsAtom, setRecordsAtom] = createSignal<ReadonlyArray<DispatchRecord>>([]);

export const dispatchRecordsAtom: Accessor<ReadonlyArray<DispatchRecord>> = recordsAtom;

/**
 * Append a dispatch to the global log. Called by each slice's dispatch
 * function. Trims to RING_CAPACITY by dropping oldest first.
 */
export function recordDispatch(record: DispatchRecord): void {
    // Read inside `untrack` so this call doesn't establish reactive
    // deps when invoked from a SolidJS reactive context (e.g. the
    // launcher-event reducer's createEffect dispatches through here).
    // Without this, the read+write pair on `recordsAtom` causes the
    // outer effect to re-run on every dispatch — observed as a
    // ~3000× runaway and renderer V8-stack crash on the storm path.
    untrack(() => {
        const prev = recordsAtom();
        const next = prev.length >= RING_CAPACITY
            ? [...prev.slice(prev.length - RING_CAPACITY + 1), record]
            : [...prev, record];
        setRecordsAtom(next);
    });
}

/**
 * Read the most recent N dispatch records, newest last. Diagnostics +
 * tests; do NOT use for app logic.
 */
export function getRecentDispatches(limit?: number): ReadonlyArray<DispatchRecord> {
    const all = recordsAtom();
    if (limit == null || limit >= all.length) return all;
    return all.slice(all.length - limit);
}

/** Test/dev helper — wipe the ring. Never call in production. */
export function __resetDispatchLog(): void {
    setRecordsAtom([]);
}

/** Convenience helpers for human-readable source descriptions. */
export function describeSource(source: CommandSource): string {
    if (source === "system") return "system";
    if (source === "user") return "user";
    return `agent:${source.agentId}`;
}
