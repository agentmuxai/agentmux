// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for the agent pane's message document.
 *
 * Pattern follows the host/launcher reducers (task #182) — pure
 * function, no I/O, snapshot input, return new state + audit events.
 * See docs/specs/agent-pane-document-reducer-2026-05-03.md.
 *
 * `nodeIdSet` is maintained as a sibling to `nodes[]` so consumers
 * (useAgentStream) can dedup new events without rebuilding the index
 * on every mount. Issue #728 gap 4.
 */

import type { DocumentNode } from "../../view/agent/types";
import {
    AgentDocumentCommand,
    AgentDocumentState,
    ReducerOptions,
    ReducerResult,
    TRUNCATE_GRACE_MS,
} from "./types";

export function update(
    state: AgentDocumentState,
    command: AgentDocumentCommand,
    /** Time injection for testability; defaults to Date.now(). */
    nowMs: number = Date.now(),
    /** Optional reducer knobs (gap 6 — injectable truncate grace). */
    opts?: ReducerOptions,
): ReducerResult {
    switch (command.type) {
        case "SessionStart":
            return {
                state: {
                    ...state,
                    sessionPhase: "active",
                    sessionStartedAt: command.at,
                },
                events: [{ type: "session-started", at: command.at }],
            };

        case "SessionEnd":
            return {
                state: { ...state, sessionPhase: "ended" },
                events: [{ type: "session-ended", at: command.at }],
            };

        case "HistoryLoaded": {
            if (command.nodes.length === 0) {
                return { state, events: [] };
            }
            const nextIdSet = new Set(state.nodeIdSet);
            const fresh: DocumentNode[] = [];
            let duplicates = 0;
            for (const n of command.nodes) {
                if (nextIdSet.has(n.id)) {
                    duplicates++;
                    continue;
                }
                nextIdSet.add(n.id);
                fresh.push(n);
            }
            if (fresh.length === 0) {
                return {
                    state,
                    events: [
                        { type: "history-loaded", addedCount: 0, duplicatesDropped: duplicates },
                    ],
                };
            }
            return {
                state: {
                    ...state,
                    nodes: [...fresh, ...state.nodes],
                    nodeIdSet: nextIdSet,
                },
                events: [
                    {
                        type: "history-loaded",
                        addedCount: fresh.length,
                        duplicatesDropped: duplicates,
                    },
                ],
            };
        }

        case "StreamFlush": {
            const noWork = command.newNodes.length === 0 && command.updatedNodes.length === 0;
            if (noWork) return { state, events: [] };

            const indexById = new Map<string, number>();
            for (let i = 0; i < state.nodes.length; i++) {
                indexById.set(state.nodes[i].id, i);
            }
            // Lazy-clone: only allocate a new array if something changes.
            let next: DocumentNode[] | null = null;
            const ensureClone = () => {
                if (!next) next = state.nodes.slice();
                return next;
            };

            // Updates first — they target IDs that are expected to exist.
            let updateApplied = 0;
            let updateDropped = 0;
            for (const upd of command.updatedNodes) {
                const idx = indexById.get(upd.id);
                if (idx == null) {
                    updateDropped++;
                    continue;
                }
                const arr = ensureClone();
                const existing = arr[idx];
                if (existing.type === "markdown" && upd.type === "markdown") {
                    arr[idx] = { ...existing, content: upd.content };
                } else {
                    arr[idx] = upd;
                }
                updateApplied++;
            }

            // New nodes: dedup against existing IDs; collisions → in-place
            // update (same protection the original `flushPendingNodes` had
            // against the rebuild-while-pending race).
            let appendedNew = 0;
            let collidedAndUpdated = 0;
            let nextIdSet: Set<string> | null = null;
            for (const n of command.newNodes) {
                const idx = indexById.get(n.id);
                if (idx != null) {
                    const arr = ensureClone();
                    arr[idx] = n;
                    collidedAndUpdated++;
                    continue;
                }
                const arr = ensureClone();
                arr.push(n);
                indexById.set(n.id, arr.length - 1);
                if (!nextIdSet) nextIdSet = new Set(state.nodeIdSet);
                nextIdSet.add(n.id);
                appendedNew++;
            }

            if (!next) {
                return { state, events: [] };
            }
            return {
                state: {
                    ...state,
                    nodes: next,
                    nodeIdSet: nextIdSet ?? state.nodeIdSet,
                },
                events: [
                    {
                        type: "stream-flushed",
                        appendedNew,
                        collidedAndUpdated,
                        updateApplied,
                        updateDropped,
                    },
                ],
            };
        }

        case "StreamTruncate": {
            const graceMs = opts?.truncateGraceMs ?? TRUNCATE_GRACE_MS;
            if (shouldSuppressTruncate(state, nowMs, graceMs)) {
                return {
                    state,
                    events: [
                        {
                            type: "truncate-suppressed",
                            reason: command.reason,
                            activeForMs: state.sessionStartedAt
                                ? nowMs - state.sessionStartedAt
                                : 0,
                            nodeCount: state.nodes.length,
                        },
                    ],
                };
            }
            const cleared = state.nodes.length;
            if (cleared === 0) {
                return { state, events: [] };
            }
            return {
                state: { ...state, nodes: [], nodeIdSet: new Set<string>() },
                events: [
                    { type: "truncate-applied", reason: command.reason, clearedCount: cleared },
                ],
            };
        }

        case "UserClear": {
            const cleared = state.nodes.length;
            return {
                state:
                    cleared === 0
                        ? state
                        : { ...state, nodes: [], nodeIdSet: new Set<string>() },
                events: [{ type: "user-cleared", clearedCount: cleared }],
            };
        }
    }
}

/**
 * Suppress truncate when:
 *   - we're in the active phase, AND
 *   - the session has been running for more than the grace window, AND
 *   - there's actual content to lose.
 *
 * Rationale: the dominant cause of mid-session truncate-wipe is a
 * socket-reconnect race. A legitimate truncate during an active,
 * established session with content is exceptionally unusual; the safe
 * default is to ignore it and keep the conversation visible. /clear is
 * the explicit way to wipe.
 */
function shouldSuppressTruncate(
    state: AgentDocumentState,
    now: number,
    graceMs: number,
): boolean {
    if (state.sessionPhase !== "active") return false;
    if (state.nodes.length === 0) return false;
    if (state.sessionStartedAt == null) return false;
    return now - state.sessionStartedAt > graceMs;
}
