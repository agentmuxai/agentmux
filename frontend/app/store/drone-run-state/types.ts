// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Type definitions for the drone-run-state reducer (slice #10 in the
 * frontend reducer roadmap). Pairs with the master reducer-stack
 * status doc and closes the "drone-model.ts is not a reducer"
 * drift item from
 * `docs/specs/SPEC_UNIFIED_AGENT_TYPES_2026_05_13.md` §5.3.
 *
 * Owns the per-drone-pane reactive cells that fold the
 * `dronerun:<id>` event stream (backend
 * `agentmux-srv/src/drone/executor/engine.rs::RunEvent`).
 * The view (`DroneViewModel`) keeps the canvas-edit cells
 * (draft graph, selection, palette interaction) — those are
 * pure UI editing state, not folded data.
 *
 * Cells owned here:
 *   - `runId` — active drone run id (`""` when no run is live).
 *   - `droneId` — id of the drone the active run was started for.
 *   - `status` — `"idle" | "running" | "done" | "failed"`.
 *   - `blockResults` — per-block last-run output keyed by block id;
 *     populated incrementally by `BlockDone` / `BlockError` events,
 *     and can be backfilled in bulk via `BackfilledFromRow` for the
 *     ultra-fast-drone race covered by PR #843.
 *   - `output` — final `RunDone.output` text (or stringified value).
 *   - `error` — final `RunFailed.error` text.
 *   - `closed` — terminal flag set by `Disposed`. After it flips,
 *     every subsequent command emits `post-close-command-dropped`.
 */

/** What we render in the Agent block inspector for the last run. Mirror
 *  of the wire `AgentRunResult` flattened into the snake_case drone
 *  block-output shape (`response`, `cost_usd`). Errors arrive via
 *  `BlockError` and reuse the same slot. */
export interface AgentBlockResult {
    response: string;
    costUsd?: number;
    error?: string;
}

export type DroneRunStatus = "idle" | "running" | "done" | "failed";

export interface DroneRunState {
    closed: boolean;
    runId: string;
    droneId: string;
    status: DroneRunStatus;
    blockResults: Record<string, AgentBlockResult>;
    output: string;
    error: string;
}

export const initialState = (): DroneRunState => ({
    closed: false,
    runId: "",
    droneId: "",
    status: "idle",
    blockResults: {},
    output: "",
    error: "",
});

/** Shape of one backfilled block row from `DroneRun.block_states`,
 *  used by `BackfilledFromRow` for the codex-P2 race recovery on PR
 *  #843. Mirror of `DroneBlockState` in `frontend/types/gotypes.d.ts`. */
interface BackfilledBlock {
    blockId: string;
    status: "pending" | "running" | "done" | "error" | "skipped";
    output?: unknown;
    error?: string;
}

export type DroneRunCommand =
    /**
     * The `Run` button kicked off a fresh drone run. Resets folded
     * state so the inspector starts clean. Fires before subscription.
     */
    | { type: "RunStarted"; runId: string; droneId: string }
    /**
     * Backend `BlockStarted` event. Currently a no-op in state — the
     * reducer just records it for the audit ring. Phase 2 polish may
     * use it to render per-block "in flight" spinners.
     */
    | { type: "BlockStarted"; blockId: string }
    /**
     * Backend `BlockDone` event. Folds `output` into
     * `blockResults[blockId]` via `parseBlockOutput`.
     */
    | { type: "BlockDone"; blockId: string; output: unknown }
    /**
     * Backend `BlockError` event. Stores the error message in
     * `blockResults[blockId]` so the inspector surfaces it.
     */
    | { type: "BlockError"; blockId: string; error: string }
    /**
     * Backend `RunDone` event. Sets status=done, copies the final
     * output. Does NOT clear blockResults — the inspector still
     * wants to show per-block results after completion.
     */
    | { type: "RunDone"; output: unknown }
    /**
     * Backend `RunFailed` event. Sets status=failed, copies the
     * error.
     */
    | { type: "RunFailed"; error: string }
    /**
     * Race recovery for ultra-fast drones: when the run finished
     * before this client subscribed, the events fired into the void
     * but the backend persisted final block states in the
     * `DroneRun` row. The view dispatches this command after
     * `refreshRuns` if the active run is already terminal so the
     * inspector still shows per-block output. Idempotent — empty
     * blocks array yields no event.
     */
    | {
          type: "BackfilledFromRow";
          runId: string;
          droneId: string;
          status: DroneRunStatus;
          output: string;
          error: string;
          blocks: BackfilledBlock[];
      }
    /**
     * View tear-down (`New drone`, open a different one). Returns
     * to the initial state but stays subscribable. Distinct from
     * `Disposed`, which is terminal.
     */
    | { type: "Reset" }
    /**
     * The pane is being torn down. After this command runs, every
     * subsequent command on this slot is a no-op. Idempotent.
     */
    | { type: "Disposed" };

export type DroneRunEvent =
    | { type: "run-started"; runId: string; droneId: string }
    | { type: "block-started"; blockId: string }
    | { type: "block-done"; blockId: string; result: AgentBlockResult }
    | { type: "block-error"; blockId: string; error: string }
    | { type: "run-done"; output: string }
    | { type: "run-failed"; error: string }
    | {
          type: "backfilled-from-row";
          runId: string;
          status: DroneRunStatus;
          blockCount: number;
      }
    | { type: "reset" }
    | { type: "disposed" }
    | { type: "post-close-command-dropped"; commandType: string };

export interface ReducerResult {
    state: DroneRunState;
    events: DroneRunEvent[];
}

/** Pull the response text + cost out of the `BlockDone.output` shape
 *  emitted by `drone/executor/blocks/agent.rs` (spec §4.3). The
 *  agent block returns `{ response, tokens, cost_usd }`; other blocks
 *  return arbitrary shapes that fall back to JSON-stringify. Pure
 *  function — safe inside the reducer + tests. */
export function parseBlockOutput(output: unknown): AgentBlockResult {
    if (output && typeof output === "object") {
        const o = output as Record<string, unknown>;
        if (typeof o["response"] === "string") {
            return {
                response: o["response"],
                costUsd:
                    typeof o["cost_usd"] === "number"
                        ? (o["cost_usd"] as number)
                        : undefined,
            };
        }
    }
    return {
        response: typeof output === "string" ? output : JSON.stringify(output ?? null),
    };
}
