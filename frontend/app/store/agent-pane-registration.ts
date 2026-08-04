// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Unified per-pane registration — atomic across BOTH stores (agent-document
 * and agent-pane-state).
 *
 * Invariant: at any observable point a blockId is either FULLY registered in
 * both stores or FULLY unregistered from both. This is the ONLY production
 * surface for register/unregister; per-store exports are `@internal — tests only`.
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
import {
    type AgentPaneModel,
    _createAgentPaneModel,
} from "./agent-pane-model";
import type { DocumentNode } from "../view/agent/types";

/**
 * Options bundle for the unified register call. Mirrors the union of
 * what the two underlying stores need:
 *
 *   - `agentId` — identity of the agent occupying this pane. Drives
 *     reducer initialState for the pane-state slot.
 *   - `documentSetter` — write-only projection for the documentAtom.
 *   - `projections` — the seven per-field setters the pane-state slot
 *     writes through (streaming / sessionStats / currentTool /
 *     turnTokens / pending / initPhase / turnPhase). PR G dropped the
 *     legacy `turnActive` and `stopping` projections — see
 *     `AgentPaneProjections` in `agent-pane-state-store.ts`.
 */
export interface PaneRegistration {
    agentId: string;
    documentSetter: (nodes: DocumentNode[]) => void;
    projections: AgentPaneProjections;
}

/**
 * Per-pane model registry. Each `registerPane` call creates one entry;
 * `unregisterPane` flips its `disposed` flag and removes it. The model
 * outlives the actual store slots by exactly long enough to ensure the
 * disposed flag is set BEFORE the underlying store deletes run (so any
 * synchronous cascade those deletes might trigger sees `disposed`
 * first).
 *
 * Internal — callers receive the model handle from `registerPane`'s
 * return value and pass it down.
 */
const paneModels = new Map<
    string,
    AgentPaneModel & { _markDisposed(): void }
>();

/** Listeners notified whenever any pane is registered or unregistered. */
const lifecycleListeners = new Set<() => void>();

/**
 * Subscribe to pane lifecycle changes (register / unregister).
 * Returns an unsubscribe function. Use in components that need to
 * keep derived data (e.g. open-definition maps) in sync with the
 * live slot set rather than relying solely on `agents:changed`.
 */
export function subscribeToPaneLifecycle(listener: () => void): () => void {
    lifecycleListeners.add(listener);
    return () => lifecycleListeners.delete(listener);
}

function notifyLifecycleListeners(): void {
    for (const l of lifecycleListeners) {
        try { l(); } catch { /* never let a subscriber break teardown */ }
    }
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
 *
 * **Returns** the per-pane `AgentPaneModel` whose lifetime matches the
 * pane. Threading the returned model into hooks/views gives them a
 * `dispatchPane` / `dispatchDoc` surface that is default-safe against
 * post-unmount dispatch races — the model carries a `disposed` flag
 * flipped before `unregisterPane` runs the store deletes. See
 * `agent-pane-model.ts` for the rationale.
 */
export function registerPane(
    blockId: string,
    reg: PaneRegistration,
): AgentPaneModel {
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

    // Re-registering a blockId (the hot-reload / re-mount case) drops
    // the prior model — its callers will keep dispatching against a
    // disposed handle, which silently drops, which is the right
    // behavior. The fresh model takes over the active dispatches.
    const prior = paneModels.get(blockId);
    if (prior) prior._markDisposed();

    const model = _createAgentPaneModel(blockId);
    paneModels.set(blockId, model);
    notifyLifecycleListeners();
    return model;
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
 * variants in the soft-dispatch path already handle correctly (they return [];
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
    // Flip the model's `disposed` flag BEFORE either store unregisters.
    // The contract: any synchronous cascade triggered by
    // a future store-delete that fires setters (the current stores
    // don't, but a future projection might) observes `model.disposed
    // === true` and the model's dispatch helpers no-op. Closes the
    // residual race where a deferred dispatcher (setTimeout / await
    // continuation) lands after the unregister starts but before it
    // completes its second store-delete.
    const model = paneModels.get(blockId);
    if (model) {
        model._markDisposed();
        paneModels.delete(blockId);
    }
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
    notifyLifecycleListeners();
}

/**
 * Diagnostic accessor for the currently-registered model. Returns
 * `null` if `blockId` is not registered. Used for tests + the rare
 * code path that doesn't receive the model from registerPane's return
 * (e.g. integration smoke that needs to peek at the live model
 * without re-registering).
 *
 * Production code that needs a model should plumb the registerPane
 * return value down through its options — that's the contract the
 * model is built around.
 */
export function getPaneModel(blockId: string): AgentPaneModel | null {
    return paneModels.get(blockId) ?? null;
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

export type { AgentPaneModel } from "./agent-pane-model";
