// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Type definitions for the agent document reducer (Option B from
 * docs/specs/agent-pane-document-reducer-2026-05-03.md).
 *
 * The reducer is a single serialized writer over the agent pane's
 * message-list state. Every mutation flows through a typed Command
 * and produces typed Events for audit. Pure function, no I/O.
 */

import type { DocumentNode } from "../../view/agent/types";

export type SessionPhase = "loading-history" | "active" | "ended";

export interface AgentDocumentState {
    nodes: DocumentNode[];
    sessionPhase: SessionPhase;
    /** ms epoch of the most recent SessionStart, or null if never started. */
    sessionStartedAt: number | null;
    /**
     * Authoritative dedup index, kept in lockstep with `nodes`. Replaces
     * the per-mount rebuild in useAgentStream that scanned `nodes[]` and
     * could miss in-flight events arriving between mount and scan. The
     * stream-side cache still exists for in-batch dedup but is now seeded
     * from this set rather than rebuilt. Issue #728 gap 4.
     */
    nodeIdSet: Set<string>;
}

export const initialState = (): AgentDocumentState => ({
    nodes: [],
    sessionPhase: "loading-history",
    sessionStartedAt: null,
    nodeIdSet: new Set<string>(),
});

/** Optional knobs the dispatcher can pass through to the reducer. */
export interface ReducerOptions {
    /**
     * Override for `TRUNCATE_GRACE_MS`. Issue #728 gap 6 — makes the grace
     * window injectable for unit tests + lets the diagnostics panel tune
     * it under load without rebuilding.
     */
    truncateGraceMs?: number;
}

/**
 * Hooks emit these. The reducer decides what to do based on current
 * state + invariants.
 */
export type AgentDocumentCommand =
    /** Hook signal: live stream subscription is up. */
    | { type: "SessionStart"; at: number }
    /** Hook signal: subscription torn down (cleanup). */
    | { type: "SessionEnd"; at: number }
    /** Persisted history (older messages from blockfile) — prepended. */
    | { type: "HistoryLoaded"; nodes: DocumentNode[] }
    /**
     * Generic merge: `newNodes` are appends (with dedup against existing
     * IDs — collisions route to in-place update), `updatedNodes` are
     * targeted mutations against existing IDs. The primary caller is the
     * stream's RAF flush, but any frontend-local mutation that needs to
     * append or update document nodes (e.g. optimistic decision-update,
     * login-success row) routes through the same command so slot.state
     * stays authoritative.
     */
    | { type: "StreamFlush"; newNodes: DocumentNode[]; updatedNodes: DocumentNode[] }
    /**
     * Backend `fileop=truncate` arrived on the file subject. The reducer
     * decides whether to honor (initialization, legitimate clear) or
     * suppress (stale event after socket reconnect — this is the bug we're
     * fixing).
     */
    | { type: "StreamTruncate"; reason: string }
    /** User invoked /clear — always honored. */
    | { type: "UserClear" };

/**
 * Audit events emitted by the reducer. v1 logs them via the dispatcher's
 * onEvent callback. The diagnostics panel surface is a follow-up.
 */
export type AgentDocumentEvent =
    | { type: "session-started"; at: number }
    | { type: "session-ended"; at: number }
    | { type: "history-loaded"; addedCount: number; duplicatesDropped: number }
    | {
          type: "stream-flushed";
          appendedNew: number;
          /** New nodes whose ID already existed → routed to in-place update. */
          collidedAndUpdated: number;
          /** Updates targeting known IDs. */
          updateApplied: number;
          /** Updates targeting unknown IDs — dropped. */
          updateDropped: number;
      }
    | { type: "truncate-applied"; reason: string; clearedCount: number }
    | {
          type: "truncate-suppressed";
          reason: string;
          /** ms since SessionStart. */
          activeForMs: number;
          /** Nodes that would have been wiped. */
          nodeCount: number;
      }
    | { type: "user-cleared"; clearedCount: number };

export interface ReducerResult {
    state: AgentDocumentState;
    events: AgentDocumentEvent[];
}

/**
 * Grace window: a `StreamTruncate` arriving in the first GRACE_MS after
 * SessionStart is honored (legitimate fresh-session reset). After that,
 * if the document is non-empty, truncate is suppressed — the most likely
 * source of a late truncate is a socket-reconnect race, and silently
 * wiping a live conversation is the bug we're fixing.
 *
 * 5000ms covers typical socket reconnect latency (1–3s) with margin.
 */
export const TRUNCATE_GRACE_MS = 5000;
