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

import type { DocumentNode, ShellNode, ToolLogChunk } from "../../view/agent/types";

type SessionPhase = "loading-history" | "active" | "ended";

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
    /**
     * id -> current index in `nodes`, kept in lockstep with `nodes`
     * (same lifecycle as `nodeIdSet`). Lets `StreamFlush` resolve
     * collisions/updates and `ToolChunkAppend` / `ShellChunkAppend` /
     * `ShellStatusUpdate` locate their target node in O(1) instead of
     * scanning the full array on every streamed chunk — task #39. Any
     * command that reorders or prepends `nodes` (`HistoryLoaded`,
     * `HistoryRestored`) must rebuild this map to match; a command that
     * only mutates node CONTENT in place (scrub, truncate-status flips)
     * leaves indices untouched and can pass the same map through.
     */
    nodeIndexById: Map<string, number>;
}

export const initialState = (): AgentDocumentState => ({
    nodes: [],
    sessionPhase: "loading-history",
    sessionStartedAt: null,
    nodeIdSet: new Set<string>(),
    nodeIndexById: new Map<string, number>(),
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
     * Restore from a snapshot file (`output.state.json`). Semantically
     * identical to `HistoryLoaded` (prepend with id-dedup), with one
     * additional effect: `sessionPhase` jumps directly to `"active"`
     * because the snapshot represents the full pre-close history —
     * there is no further history to page in.
     *
     * Why prepend (not full-replace): `useAgentStream` may dispatch
     * `StreamFlush` during the async snapshot read window. A full
     * replace would wipe those live arrivals. Existing nodes (live
     * arrivals) win on id collision; the snapshot version is dropped.
     *
     * `fromSnapshot: true` is a discriminator field per spec §4.5 — the
     * view layer reads it from the audit event (history-restored) to
     * distinguish snapshot restore from partial `HistoryLoaded` prepend,
     * and suppress the "Loading older messages" affordance.
     *
     * Spec: docs/specs/SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md §4.5.
     */
    | { type: "HistoryRestored"; nodes: DocumentNode[]; fromSnapshot: true }
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
     * Append one streaming chunk to a tool's live-log buffer
     * (`ToolNode.log.chunks`). Append-only, idempotent on the timestamp
     * + content dedup key. Dispatched at high frequency from
     * `useAgentStream` for `tool_chunk` stream events — kept distinct
     * from `StreamFlush` so chunk traffic doesn't re-trigger memos
     * that watch the full node array length. See
     * SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md §3.3.
     */
    | { type: "ToolChunkAppend"; toolId: string; chunk: ToolLogChunk }
    /** Create a new ShellNode in the document (fired on shell_node_create WPS event). */
    | { type: "ShellNodeCreate"; node: ShellNode }
    /** Append a streaming chunk to a ShellNode's live-log. */
    | { type: "ShellChunkAppend"; shellId: string; chunk: ToolLogChunk }
    /** Update a ShellNode's terminal status (exit, stop). */
    | { type: "ShellStatusUpdate"; shellId: string; status: ShellNode["status"]; exitCode?: number; exitedAt: number }
    /**
     * Backend `fileop=truncate` arrived on the file subject. The reducer
     * decides whether to honor (initialization, legitimate clear) or
     * suppress (stale event after socket reconnect — this is the bug we're
     * fixing).
     */
    | { type: "StreamTruncate"; reason: string }
    /** User invoked /clear — always honored. */
    | { type: "UserClear" }
    /**
     * Scrub orphaned in-progress nodes. Walks `nodes[]` and:
     *  - flips any markdown with `metadata.thinking === true` to
     *    `metadata.canceled === true` (clearing the thinking flag).
     *  - flips any `tool` with `status === "running"` to
     *    `status: "canceled"`.
     *
     * Dispatched as a side-effect of `SessionEnd` (clean exits)
     * and `HistoryRestored` / `HistoryLoaded` (resumed sessions
     * where the kill happened before `SessionEnd` ever fired).
     * Idempotent — running it twice is a no-op against the
     * already-scrubbed state.
     *
     * `scope: "tools-only"` narrows the sweep to `tool` nodes with
     * `status === "running"` — dispatched shortly after every TurnEnd
     * (agent-view.tsx): a foreground tool call cannot outlive its turn
     * (it blocks it) and a backgrounded harness call resolves its
     * ToolNode immediately, so a running tool node observed after the
     * turn ended is provably an orphan (rejected call / dropped
     * tool_result). Thinking markdown, shells (turn-independent — a
     * live MCP shell legitimately spans turns), and awaiting_answer
     * questions are deliberately untouched in this scope; their
     * lifecycles are session-bounded, not turn-bounded.
     *
     * Spec: docs/specs/SPEC_ORPHAN_THINKING_NODES_2026_05_27.md.
     */
    | { type: "ScrubOrphanedInProgress"; at: number; scope?: "tools-only" }
    /**
     * Force one specific `ToolNode` to `status: "canceled"`, regardless of
     * how long it's been running or whether a session boundary has
     * happened. Dispatched when a `dock:clear` WPS event arrives (a
     * `muxspect dock clear` request for this pane's block) — the manual
     * escape hatch for a stuck node, distinct from `ScrubOrphanedInProgress`'s
     * blanket sweep. No-op (empty events) if `nodeId` isn't a currently
     * tracked tool node — already resolved, or a stale/duplicate event.
     * Spec: docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md §3.2.
     */
    | { type: "ForceCancelToolNode"; nodeId: string };

/**
 * Audit events emitted by the reducer. v1 logs them via the dispatcher's
 * onEvent callback. The diagnostics panel surface is a follow-up.
 */
export type AgentDocumentEvent =
    | { type: "session-started"; at: number }
    | { type: "session-ended"; at: number }
    | { type: "history-loaded"; addedCount: number; duplicatesDropped: number }
    | { type: "history-restored"; restoredCount: number; fromSnapshot: true }
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
    | {
          type: "tool-chunk-appended";
          toolId: string;
          /** Chunks attached to this tool AFTER the append (>=1). */
          chunkCount: number;
      }
    | {
          type: "tool-chunk-dropped";
          toolId: string;
          /** Reason the chunk was rejected without touching state. */
          reason: "unknown-tool-id" | "node-not-tool" | "duplicate";
      }
    | { type: "user-cleared"; clearedCount: number }
    | {
          /**
           * The working scrollback was clamped at a `session_outcome`/
           * `fresh` boundary: `removedCount` nodes strictly older than the
           * boundary were dropped from the visible document (the persisted
           * stream is untouched — display scope only). Emitted by
           * HistoryRestored / HistoryLoaded / StreamFlush via
           * `clampToSessionScope`. Spec:
           * SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md §3.
           */
          type: "session-scope-trimmed";
          removedCount: number;
      }
    | {
          type: "orphans-scrubbed";
          /** Thinking-markdown nodes flipped to canceled. */
          markdownCanceled: number;
          /** Tool nodes whose status flipped from "running" → "canceled". */
          toolsCanceled: number;
          /**
           * Every tool node whose status this scrub actually changed —
           * `agent-document-store.ts`'s `dispatch()` pushes a final
           * `docknodestatus` delta for each so the srv-side `muxspect dock`
           * cache learns about the resolution instead of reporting an
           * already-resolved node as stuck forever (reagentx P1, PR #2432).
           */
          resolvedToolNodes: Array<{ id: string; status: string; toolName: string }>;
      }
    | { type: "tool-force-canceled"; nodeId: string }
    | {
          type: "tool-force-cancel-skipped";
          nodeId: string;
          /** Reason the target node wasn't found — same taxonomy as tool-chunk-dropped. */
          reason: "unknown-tool-id" | "node-not-tool";
      };

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
