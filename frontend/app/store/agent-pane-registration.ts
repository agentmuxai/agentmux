// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Unified per-pane registration — atomic across BOTH stores.
 *
 * PR-3 of the cascade follow-up sequence (see
 * docs/analysis/LIFECYCLE_DISPATCH_LEAK_2026_05_15.md §6.2 + the
 * 2026-05-23 retro on the replaceChild cascade incident).
 *
 * ## The problem this closes
 *
 * Before this module existed, agent-view registered each pane with two
 * stores in sequence:
 *
 *     registerAgentDocPane(blockId, setter);                       // store A
 *     registerAgentPaneStatePane(blockId, agentId, projections);   // store B
 *
 * There was a brief synchronous window AFTER store A registered and
 * BEFORE store B registered — and an even worse asymmetry on cleanup
 * (the unregister calls happen in two distinct `onCleanup` callbacks
 * registered against the same owner). A `StreamFlush` dispatch (from
 * `useAgentStream`, hooked subscriptions, or anywhere else with a hot
 * dispatch loop) that landed in either window found the pane registered
 * in one store but not the other.
 *
 * That partial-registration window is precisely the cascade-mid-dispatch
 * failure mode PR #878 added detection for and PR #989 migrated three
 * dispatch sites away from throw-on-missing. PR-3 closes the window
 * structurally — by making registration / unregistration atomic across
 * both stores at the source.
 *
 * ## The contract
 *
 * At any observable point in time (from any dispatcher's POV), a blockId
 * is either FULLY registered in BOTH stores, or FULLY unregistered from
 * BOTH stores. There is no half-state.
 *
 * Practically: this module is the ONLY surface that production code uses
 * to register / unregister a pane. The per-store `registerPane` /
 * `unregisterPane` exports survive but are documented `@internal — tests
 * only`. The store-internal tests
 * (`agent-pane-state-store.test.ts`, `agent-document-store.test.ts`)
 * exercise each reducer in isolation and need direct register/unregister
 * access without bringing the sibling store along; they import the
 * per-store functions directly. Every production call site goes through
 * this helper.
 *
 * ## Option A/B choice
 *
 * - **Option A** (keep both per-store `registerPane` exported, mark
 *   `@internal`): lighter, enforcement is comment-only.
 * - **Option B** (per-store registration is private to the store + the
 *   unified helper; tests use the helper too): strictest.
 *
 * We picked **Option A** for a pragmatic reason: the per-store cascade
 * tests in `agent-pane-state-store.test.ts` (PR #878) drive
 * single-store cascade-disposal scenarios that require direct access to
 * `registerPane` / `unregisterPane` on a single store, with custom
 * projection setters that synchronously call `unregisterPane` to
 * simulate a reactive subscriber's mid-dispatch dispose. Forcing those
 * tests through the unified helper would require either bringing up the
 * full second-store slot (changing what the test is testing) or adding
 * a test-only escape hatch (which defeats Option B's enforcement
 * point). Marking the per-store exports `@internal — tests only` and
 * documenting that ALL production callers go through the unified helper
 * gives the same structural guarantee for the actual failure mode (the
 * half-registered window in agent-view's lifecycle) without the test
 * thrash.
 *
 * If the test surface ever migrates to a fake/mock projection layer
 * that doesn't need raw single-store access, the per-store exports can
 * be flipped to Option-B-strict (renamed `_register*` etc.) in a
 * follow-up. PR-4 of the cascade sequence introduces a model-level
 * `dispatchIfAlive` pattern that may make the per-store cascade tests
 * obsolete — at which point the Option B flip becomes cheap.
 */

import {
    registerPane as registerAgentDocPaneRaw,
    snapshot as agentDocSnapshot,
    unregisterPane as unregisterAgentDocPaneRaw,
} from "./agent-document-store";
import {
    type AgentPaneProjections,
    registerPane as registerAgentPaneStatePaneRaw,
    snapshot as agentPaneStateSnapshot,
    unregisterPane as unregisterAgentPaneStatePaneRaw,
} from "./agent-pane-state-store";
import type { DocumentNode } from "../view/agent/types";

/**
 * Options bundle for the unified register call. Mirrors the union of
 * what the two underlying stores need:
 *
 *   - `agentId` — identity of the agent occupying this pane. Drives
 *     reducer initialState for the pane-state slot.
 *   - `documentSetter` — write-only projection for the documentAtom.
 *   - `projections` — the eight per-field setters the pane-state slot
 *     writes through (streaming / sessionStats / currentTool /
 *     turnTokens / turnActive / stopping / pending / initPhase /
 *     turnPhase).
 */
export interface PaneRegistration {
    agentId: string;
    documentSetter: (nodes: DocumentNode[]) => void;
    projections: AgentPaneProjections;
}

/**
 * Atomically register a pane in BOTH stores. Either both succeed or
 * neither does — if the second store throws during registration, the
 * first is rolled back so observers never see the half-state.
 *
 * Idempotent in the same sense as the underlying stores: re-registering
 * a blockId resets state in both slots.
 *
 * MUST be called synchronously from the agent component body, before
 * any hook can dispatch. See `agent-view.tsx`.
 */
export function registerPane(blockId: string, reg: PaneRegistration): void {
    // Register both stores synchronously. JS is single-threaded — no
    // dispatcher can observe the gap between these two lines unless one
    // of them triggers a reactive cascade. Both register functions are
    // a simple `Map.set` with no setter calls, so they cannot themselves
    // cascade-dispatch.
    registerAgentDocPaneRaw(blockId, reg.documentSetter);
    try {
        registerAgentPaneStatePaneRaw(blockId, reg.agentId, reg.projections);
    } catch (e) {
        // Pane-state register failed — roll back the document slot so
        // the contract holds. The document slot was set above; we own
        // the lifecycle here.
        try {
            unregisterAgentDocPaneRaw(blockId);
        } catch {
            // Best-effort rollback. If unregister itself throws (the
            // current stores never do), we accept a leaked document
            // slot over a partially-registered pane — but the throw
            // below still surfaces the root failure.
        }
        throw e;
    }
}

/**
 * Atomically unregister a pane from BOTH stores. Both unregister calls
 * complete before this function returns — no dispatcher can observe the
 * pane registered in one store but not the other across this boundary.
 *
 * Idempotent: calling unregister on a blockId that isn't registered in
 * either store is a no-op (each underlying `Map.delete` returns false
 * silently).
 *
 * Order: pane-state FIRST, then document. The cascade scenario this
 * defends against is a documentAtom subscriber that unmounts the pane
 * during a document setter call (the Shape B cascade from the
 * LIFECYCLE_DISPATCH_LEAK analysis). The intermediate state seen by
 * any synchronously-running subscriber between the two `delete` calls
 * is "pane-state gone, document still there" — which the soft-dispatch
 * variants in PR #878 + #989 already handle correctly (they return [];
 * they don't throw). Reversing the order would expose the
 * "document gone, pane-state still there" intermediate, in which a
 * pane-state subscriber that read the document store would see an
 * inconsistent view.
 *
 * Per-store unregister failures are swallowed so one failing teardown
 * can't strand the other slot. The current stores never throw on
 * unregister, but the cross-store atomicity contract belongs to the
 * helper, not the callers.
 */
export function unregisterPane(blockId: string): void {
    try {
        unregisterAgentPaneStatePaneRaw(blockId);
    } catch {
        // Best-effort — never let the first teardown strand the second.
    }
    try {
        unregisterAgentDocPaneRaw(blockId);
    } catch {
        // Best-effort — see above.
    }
}

/**
 * Diagnostic — true only when the pane is registered in BOTH stores.
 * Any other state (registered in only one, registered in neither)
 * returns `false`. That's the invariant the unified helper exists to
 * preserve. Used by the registration-invariant test.
 */
export function isPaneFullyRegistered(blockId: string): boolean {
    return (
        agentDocSnapshot(blockId) !== null &&
        agentPaneStateSnapshot(blockId) !== null
    );
}

/**
 * Diagnostic — true if the pane is registered in EXACTLY ONE of the two
 * stores (the failure mode this module exists to prevent). Used by the
 * registration-invariant test.
 */
export function isPaneHalfRegistered(blockId: string): boolean {
    const inDoc = agentDocSnapshot(blockId) !== null;
    const inPaneState = agentPaneStateSnapshot(blockId) !== null;
    return inDoc !== inPaneState;
}

export type { AgentPaneProjections };
