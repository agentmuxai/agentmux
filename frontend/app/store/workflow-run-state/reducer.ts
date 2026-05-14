// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for the workflow-run-state slice (#10).
 *
 * Mirrors slice #9 (browser-pane-state) — same `update(state, command) →
 * { state, events }` shape, same idempotency rules, same no-throw policy,
 * same post-close-command-dropped gate. See `types.ts` for cell-level
 * documentation.
 *
 * Invariants:
 *   1. Once `closed` flips true, every subsequent command emits
 *      `post-close-command-dropped` and returns the unchanged state.
 *      `Disposed` itself is idempotent (second dispatch is a no-op).
 *   2. `RunStarted` resets every folded cell (blockResults, output,
 *      error) — the inspector restarts clean each run.
 *   3. `BlockDone` / `BlockError` write to `blockResults[blockId]`
 *      atomically. `BlockStarted` is a record-only event (no state
 *      change today).
 *   4. `RunDone` / `RunFailed` flip status to terminal but do NOT
 *      clear `blockResults` — the inspector still wants to render
 *      per-block output after completion.
 *   5. `BackfilledFromRow` only writes blocks that have a meaningful
 *      output/error (status === "done" or "error"); other states are
 *      skipped. It overwrites existing `blockResults` because backfill
 *      is the authoritative final state from persistence — any partial
 *      data accumulated mid-run is stale.
 *   6. `Reset` returns to `initialState()` (preserves `closed=false`).
 */

import {
    initialState,
    parseBlockOutput,
    ReducerResult,
    WorkflowRunCommand,
    WorkflowRunState,
} from "./types";

export function update(
    state: WorkflowRunState,
    command: WorkflowRunCommand,
): ReducerResult {
    if (state.closed && command.type !== "Disposed") {
        return {
            state,
            events: [
                {
                    type: "post-close-command-dropped",
                    commandType: command.type,
                },
            ],
        };
    }

    switch (command.type) {
        case "RunStarted": {
            return {
                state: {
                    ...state,
                    runId: command.runId,
                    workflowId: command.workflowId,
                    status: "running",
                    blockResults: {},
                    output: "",
                    error: "",
                },
                events: [
                    {
                        type: "run-started",
                        runId: command.runId,
                        workflowId: command.workflowId,
                    },
                ],
            };
        }

        case "BlockStarted": {
            return {
                state,
                events: [{ type: "block-started", blockId: command.blockId }],
            };
        }

        case "BlockDone": {
            const result = parseBlockOutput(command.output);
            return {
                state: {
                    ...state,
                    blockResults: {
                        ...state.blockResults,
                        [command.blockId]: result,
                    },
                },
                events: [
                    { type: "block-done", blockId: command.blockId, result },
                ],
            };
        }

        case "BlockError": {
            return {
                state: {
                    ...state,
                    blockResults: {
                        ...state.blockResults,
                        [command.blockId]: { response: "", error: command.error },
                    },
                },
                events: [
                    { type: "block-error", blockId: command.blockId, error: command.error },
                ],
            };
        }

        case "RunDone": {
            const output =
                typeof command.output === "string"
                    ? command.output
                    : JSON.stringify(command.output ?? null);
            return {
                state: {
                    ...state,
                    status: "done",
                    output,
                },
                events: [{ type: "run-done", output }],
            };
        }

        case "RunFailed": {
            return {
                state: {
                    ...state,
                    status: "failed",
                    error: command.error,
                },
                events: [{ type: "run-failed", error: command.error }],
            };
        }

        case "BackfilledFromRow": {
            if (command.blocks.length === 0 && state.status === command.status) {
                return { state, events: [] };
            }
            const blockResults: Record<string, ReturnType<typeof parseBlockOutput>> = {};
            for (const b of command.blocks) {
                if (b.status === "done") {
                    blockResults[b.blockId] = parseBlockOutput(b.output);
                } else if (b.status === "error") {
                    blockResults[b.blockId] = {
                        response: "",
                        error: b.error ?? "block failed",
                    };
                }
            }
            return {
                state: {
                    ...state,
                    runId: command.runId,
                    workflowId: command.workflowId,
                    status: command.status,
                    blockResults,
                    output: command.output,
                    error: command.error,
                },
                events: [
                    {
                        type: "backfilled-from-row",
                        runId: command.runId,
                        status: command.status,
                        blockCount: Object.keys(blockResults).length,
                    },
                ],
            };
        }

        case "Reset": {
            return {
                state: { ...initialState(), closed: state.closed },
                events: [{ type: "reset" }],
            };
        }

        case "Disposed": {
            if (state.closed) return { state, events: [] };
            return {
                state: { ...state, closed: true },
                events: [{ type: "disposed" }],
            };
        }
    }
}
