// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Bounded retry ladder for the auto-unblock auth recheck fired by
 * `agentidentities:changed` — SPEC_AGENT_LOGIN_FLOW_TIGHTENING_2026_09_04.md
 * §2.2, and the fix for a P1 caught on PR #2969's first review pass:
 *
 * The backend publishes `agentidentities:changed:<agentId>` SYNCHRONOUSLY
 * inside the `LinkAgentIdentityCommand` handler, before it even responds to
 * the RPC (`agent_handlers/identity.rs:590-611`). RPC responses and WS
 * events share one in-order connection, so the frontend's subscription
 * fires before `bindAccountToAgent`'s own `SetMetaCommand` — which only
 * runs AFTER that same Link RPC resolves client-side — has refreshed
 * `cmd:env` to the newly-bound account's dir. A single, immediate recheck
 * therefore reads STALE env and fails on essentially every bind, not as an
 * edge case but as the common one.
 *
 * `recheck` must re-read fresh state on every call (never a cached
 * snapshot — `useAgentControllerStatus.recheckAuthAfterBind` already does
 * this by construction) for retrying to actually converge: each attempt
 * picks up whatever `cmd:env` holds AT THAT MOMENT, so it naturally
 * succeeds the instant the real refresh lands, typically within one tick.
 *
 * Extracted as a pure, dependency-injected function (rather than left
 * inline in agent-view.tsx) because that exact "unassertable inline logic"
 * shape is what `PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02.md`'s own
 * retrospective flagged as the reason the synthetic-row effect had to be
 * extracted to `synthetic-row.ts` after several P1s — no sense re-learning
 * that lesson on the same file.
 */

export interface RetryRecheckAfterBindDeps {
    /** Re-run the auth check against current state; true = now authenticated. */
    recheck: () => Promise<boolean>;
    /** Whether the pane is still in a state worth continuing to poll for
     *  (canRetry, or a live "auth" failure). Checked BETWEEN attempts so a
     *  ladder in flight bails early if something else already resolved it
     *  (e.g. a live turn arriving — see agent-view.tsx's
     *  onActiveTurnConfirmed) or the row was dismissed. */
    stillBlocked: () => boolean;
    sleep: (ms: number) => Promise<void>;
    /** Fired exactly once, the moment `recheck` first returns true. */
    onHealthy: () => void;
}

export const DEFAULT_RECHECK_AFTER_BIND_DELAYS_MS = [300, 700, 1500];

/**
 * Try `recheck` up to `delaysMs.length + 1` times, sleeping `delaysMs[i]`
 * between attempt `i` and `i+1`, stopping early on success or on
 * `stillBlocked()` turning false. Returns whether it ended healthy.
 */
export async function retryRecheckAfterBind(
    deps: RetryRecheckAfterBindDeps,
    delaysMs: number[] = DEFAULT_RECHECK_AFTER_BIND_DELAYS_MS,
): Promise<boolean> {
    for (const delay of delaysMs) {
        if (await deps.recheck()) {
            deps.onHealthy();
            return true;
        }
        await deps.sleep(delay);
        if (!deps.stillBlocked()) return false;
    }
    // Final attempt after the last delay — kept explicit rather than folded
    // into the loop so `delaysMs` reads as "N delays between N+1 attempts,"
    // not "N attempts then N delays."
    if (await deps.recheck()) {
        deps.onHealthy();
        return true;
    }
    return false;
}
