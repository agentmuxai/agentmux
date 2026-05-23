// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentPaneModel — per-pane lifecycle handle with a `disposed` flag.
 *
 * PR-4 of the cascade follow-up sequence (see
 * docs/retro/retro-agent-pane-cascade-replacechild-2026-05-23.md §"Action
 * items" → "PR-4", + docs/analysis/LIFECYCLE_DISPATCH_LEAK_2026_05_15.md).
 *
 * ## What this adds
 *
 * Each agent pane gets a model object whose lifetime matches the pane
 * itself: created during `registerPane`, marked `disposed` at the very
 * START of `unregisterPane` (before either store deletes its slot), and
 * carried by props/opts into the hooks that need to dispatch.
 *
 * Hooks that previously had to remember "use `dispatchIfRegistered`
 * here because it's an async callsite that can race against unmount"
 * call `model.dispatchPane(cmd)` / `model.dispatchDoc(cmd)` instead. The
 * disposed-flag check inside the model is the structural safeguard —
 * new code is default-safe, no need for a reviewer to spot the soft
 * variant choice case-by-case.
 *
 * ## Pattern source
 *
 * `frontend/app/view/drone/drone-model.ts` (see `DroneViewModel.disposed`
 * + `DroneViewModel.dispatchIfAlive` at lines ~131 and 509–515): same
 * idempotency-flag rule applied to a single-store handle. PR-4
 * generalizes it across both agent-pane stores in one helper.
 *
 * ## Coexistence with module-level `dispatchIfRegistered`
 *
 * We picked **Option B-coexist**: the per-store `dispatchIfRegistered`
 * exports stay live. Reasons:
 *
 *   - Some dispatchers don't have a model handle (the cascade-detection
 *     log inside the stores themselves; tests; future code that doesn't
 *     thread the model in).
 *   - Migration can land incrementally — each hook that gains a
 *     `model` opt flips its own dispatches.
 *
 * The model is the PREFERRED path; new code defaults to it. The
 * cleanup PR that removes `dispatchIfRegistered` is downstream, gated
 * on every production call site migrating.
 *
 * ## Why this AND the per-store cascade detection
 *
 * The cascade-detection logging in agent-pane-state-store.ts (PR #878)
 * + the unified atomic registration (PR #999) close the cascade source.
 * The model-level disposed flag is a SECOND safety net: even if some
 * future code path manages to schedule a dispatch that lands AFTER the
 * cleanup runs (a stale setTimeout, an unhandled promise resolution, a
 * subscribe handler still in flight), the disposed check catches it
 * and turns it into a fire-and-forget no-op with one debug log line.
 *
 * The two safety nets compose: cascade detection identifies the trigger
 * setter when a dispatch lands mid-frame; the disposed flag stops a
 * post-cleanup dispatch from reaching the underlying store at all.
 */

import {
    type AgentDocumentCommand,
    type AgentDocumentEvent,
    dispatchIfRegistered as dispatchDocIfRegisteredRaw,
} from "./agent-document-store";
import {
    type AgentPaneCommand,
    type AgentPaneEvent,
    dispatchIfRegistered as dispatchPaneIfRegisteredRaw,
} from "./agent-pane-state-store";
import type { CommandSource } from "./command-source";

/**
 * Per-pane handle. One instance is created during `registerPane` (from
 * agent-pane-registration.ts) and lives until `unregisterPane`. Threaded
 * into hooks/views as `opts.model` so they can dispatch against the
 * agent pane's two stores without having to remember the soft-variant
 * rule case-by-case.
 *
 * Construction is internal to agent-pane-registration.ts —
 * `_createAgentPaneModel` below is `@internal`. Callers receive the
 * model from `registerPane` and pass it down.
 */
export interface AgentPaneModel {
    /** Stable id of the pane this model belongs to. */
    readonly blockId: string;

    /**
     * `true` iff `unregisterPane` has been called for this model's
     * pane. Set BEFORE the underlying store unregisters run, so any
     * reactive cascade those unregisters might trigger (currently they
     * don't fire setters, but the contract holds for any future store
     * that does) observes `disposed === true` and the model's dispatch
     * helpers no-op.
     *
     * Read via a getter so the flag is checked at dispatch time, not
     * at model-construction time. A signal accessor would also work,
     * but `disposed` doesn't need reactivity — nothing renders off
     * it; dispatchers just gate-check.
     */
    readonly disposed: boolean;

    /**
     * Dispatch a command against the pane-state store. Default-safe:
     * - If `disposed`, drop with no throw and one debug log. Returns
     *   an empty event array.
     * - Otherwise forward to the per-store `dispatchIfRegistered`
     *   (soft variant), which itself drops silently if the slot is
     *   somehow already gone (race between disposed-flag flip and
     *   underlying delete is closed because we flip the flag FIRST in
     *   `_markDisposed`).
     *
     * Returns the audit-event array the reducer produced, identical
     * to the underlying `dispatchIfRegistered`. Most callers don't
     * need the return — but `useAgentStream`'s StreamTruncate path
     * branches on whether the reducer emitted `truncate-applied`, so
     * preserving the event return keeps that one inspection working
     * without a separate snapshot read.
     */
    dispatchPane(
        command: AgentPaneCommand,
        source?: CommandSource,
    ): AgentPaneEvent[];

    /**
     * Same as `dispatchPane`, but for the document store. Returns
     * the audit-event array the reducer produced.
     */
    dispatchDoc(
        command: AgentDocumentCommand,
        source?: CommandSource,
    ): AgentDocumentEvent[];
}

/**
 * The concrete model — kept as a class so `instanceof` is cheap if a
 * future helper wants to discriminate, and so the disposed flag is
 * encapsulated (no caller can flip it from outside the module via
 * a plain object write).
 */
class AgentPaneModelImpl implements AgentPaneModel {
    readonly blockId: string;
    private _disposed = false;

    constructor(blockId: string) {
        this.blockId = blockId;
    }

    get disposed(): boolean {
        return this._disposed;
    }

    dispatchPane(
        command: AgentPaneCommand,
        source: CommandSource = "system",
    ): AgentPaneEvent[] {
        if (this._disposed) {
            // Single debug log per drop — gives a forensic trail for
            // "dispatch dropped because pane disposed" without spamming
            // a busy stream. Production builds typically strip debug
            // by default; the line still helps in dev.
            console.debug(
                `[agent-pane-model] dispatchPane dropped: pane ${this.blockId.slice(0, 7)} disposed (cmd=${command.type}, source=${source})`,
            );
            return [];
        }
        return dispatchPaneIfRegisteredRaw(this.blockId, command, source);
    }

    dispatchDoc(
        command: AgentDocumentCommand,
        source: CommandSource = "system",
    ): AgentDocumentEvent[] {
        if (this._disposed) {
            console.debug(
                `[agent-pane-model] dispatchDoc dropped: pane ${this.blockId.slice(0, 7)} disposed (cmd=${command.type}, source=${source})`,
            );
            return [];
        }
        return dispatchDocIfRegisteredRaw(this.blockId, command, source);
    }

    /**
     * Flip the disposed flag. Called by `agent-pane-registration.ts`'s
     * `unregisterPane` BEFORE either underlying store unregisters, so
     * any synchronous cascade those unregisters might trigger observes
     * the disposed state via this model first.
     *
     * @internal — exposed for the registration helper only.
     */
    _markDisposed(): void {
        this._disposed = true;
    }
}

/**
 * Construct a new model. `@internal` — only `agent-pane-registration.ts`
 * should call this. Callers receive their `AgentPaneModel` from
 * `registerPane`.
 */
export function _createAgentPaneModel(blockId: string): AgentPaneModel & {
    _markDisposed(): void;
} {
    return new AgentPaneModelImpl(blockId);
}
