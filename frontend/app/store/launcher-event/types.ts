// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Types for the launcher-event reducer (slice #6 in
 * docs/specs/frontend-reducer-architecture-2026-05-03.md).
 *
 * Pattern follows the conventions established in
 * docs/specs/frontend-reducer-conventions-2026-05-03.md. Single global
 * slice (no per-key slot map — there's only one launcher state).
 *
 * History context (kept for the reviewer):
 * - Originally written for Phase B.7.3.
 * - Phase B.7.3.2 (PR #603) made typed events authoritative for the
 *   InstancePanel atoms; bespoke channel was demoted to fallback.
 * - Phase B.7.3.3 (PR #604) retired the bespoke channel entirely.
 * - This refactor (PR-B in the implementation plan) extracts the pure
 *   reducer + adds backfilled tests; no behavior change.
 */

import type { LauncherEvent } from "@/util/launcher-events";
import type { WindowEntry } from "@/app/store/global";

export type { WindowEntry };

/**
 * State the launcher-event reducer owns. All four fields are immutable
 * — each command application produces a new object (or returns the same
 * reference when nothing changed, per conventions §3).
 */
export interface LauncherEventState {
    /**
     * Mirror of label → WindowEntry for top-level + sub windows. Pool
     * labels (`window-pool-*`) and browser-pane labels are excluded by
     * `isInstanceLabel` before they reach the reducer.
     */
    knownEntries: ReadonlyMap<string, WindowEntry>;
    /**
     * True once `ApplySeed` has run. Until then, close events are
     * recorded as tombstones in `closedBeforeSeed` (codex P2 #603).
     */
    seedHasHappened: boolean;
    /**
     * Tombstone set: labels for which a Close event arrived BEFORE
     * seed ran. Drained at seed time. Stays `null` after seed (close
     * events apply directly to `knownEntries`).
     */
    closedBeforeSeed: ReadonlySet<string> | null;
    /**
     * Derived from `knownEntries`: filtered + sorted view that the
     * projection layer writes to atoms. Stored on state so consumers
     * (and tests) get a single coherent snapshot per command.
     */
    instances: ReadonlyArray<WindowEntry>;
}

export const initialState = (): LauncherEventState => ({
    knownEntries: new Map(),
    seedHasHappened: false,
    closedBeforeSeed: new Set(),
    instances: [],
});

/**
 * Two command shapes feed the reducer:
 *   1. `ApplyEvent` — typed event from the launcher channel
 *   2. `ApplySeed` — RPC-snapshot seed at app init
 */
export type LauncherEventCommand =
    | { type: "ApplyEvent"; event: LauncherEvent }
    | { type: "ApplySeed"; entries: ReadonlyArray<WindowEntry> };

/**
 * Audit events emitted by the reducer. Useful for tests + future
 * diagnostics surface; the dispatch layer logs notable ones (drift,
 * saga events) to console as before.
 */
export type LauncherEventReducerEvent =
    | { type: "window-opened"; label: string; preservedWindowId: boolean }
    | {
          type: "window-closed";
          label: string;
          tombstoned: boolean;
          deletedFromKnown: boolean;
      }
    | { type: "window-instance-assigned"; label: string; createdMissing: boolean }
    | {
          type: "window-instance-released";
          label: string;
          tombstoned: boolean;
          deletedFromKnown: boolean;
      }
    | {
          type: "backend-window-id-registered";
          label: string;
          windowId: string | null;
          changed: boolean;
      }
    | { type: "backend-window-id-unregistered"; label: string }
    | { type: "drift-detected"; eventName: string }
    | { type: "saga-event-observed"; eventName: string }
    | { type: "unknown-variant-ignored"; eventName: string }
    | { type: "filtered-out"; label: string; reason: string }
    | { type: "seeded"; addedCount: number; tombstonesSkipped: number };

export interface ReducerResult {
    state: LauncherEventState;
    events: LauncherEventReducerEvent[];
}

/**
 * Filter for labels the InstancePanel surfaces. Sub-windows + main +
 * full instances all match `/^window-/` or === "main". Pool windows and
 * browser-pane child HWNDs are excluded — pool windows fire different
 * event variants that this reducer doesn't subscribe to, and
 * browser-pane HWNDs never appear as `Event::WindowOpened`.
 *
 * Exported via the store layer for callers that need the same filter
 * outside the reducer (notably `app-init.ts::initInstanceTracking`).
 * The two filters MUST stay in sync — past divergence on `window-pool-*`
 * was caught by reagent P2 #603.
 */
export function isInstanceLabel(label: string): boolean {
    if (label === "main") return true;
    if (label.startsWith("window-pool-")) return false;
    if (label.startsWith("browser-pane-")) return false;
    return label.startsWith("window-");
}
