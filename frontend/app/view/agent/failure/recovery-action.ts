// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * recovery-action — what a pane should do after a provider login succeeds.
 *
 * Extracted as a named, tested decision because it is subtler than it looks
 * and has already produced two P1s on PR #2951:
 *
 *   - Retrying unconditionally resends an old transcript message on an agent
 *     that never ran a turn (codex).
 *   - Skipping unconditionally leaves a genuinely never-launched agent
 *     authenticated with a running controller that never received its startup
 *     payload — identity, instructions, context (reagent). That is the same
 *     bug reagent/codex caught on `relogin` in PR #2318, and `onReadyFn` is
 *     the ONLY path that ever delivers it.
 *
 * `relogin` already encodes this distinction via its `retryAfterLogin`
 * argument (`useAgentControllerStatus.ts:745/805`). `loginViaTerminal`'s
 * `terminal-success` branch does not — it calls `onRecovered` unconditionally
 * — so the caller has to make the decision from the failure itself. That path
 * only became reachable for the pre-launch case when
 * PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02 consolidated onto the shared
 * failure row, which exposes "Login via terminal" as a secondary action; the
 * old standalone blue bar offered only "Log in".
 */

import type { PaneFailure } from "@/app/store/agent-pane-state/types";

/**
 * - `retry-turn`   — a turn ran and failed on auth; re-run it now that the
 *                    credential is good.
 * - `send-startup` — no turn ever ran (the pre-launch case). There is nothing
 *                    to retry; the agent needs its startup sequence instead.
 *                    Safe even when a session already exists, because
 *                    `onReadyFn` self-guards on `agent:sessionid`.
 */
export type PostLoginRecovery = "retry-turn" | "send-startup";

/**
 * Decide from the failure that was showing when recovery succeeded.
 *
 * Defaults to `retry-turn` when the flag is absent — every backend-classified
 * failure is by construction the outcome of a turn that ran, and that was the
 * only behaviour before `turnAttempted` existed.
 */
export function postLoginRecoveryFor(failure: PaneFailure | null | undefined): PostLoginRecovery {
    return failure?.turnAttempted === false ? "send-startup" : "retry-turn";
}
