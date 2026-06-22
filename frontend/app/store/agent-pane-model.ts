// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentPaneModel — per-pane lifecycle handle that carries a `disposed` flag
 * and dispatch helpers for both agent-pane stores.
 *
 * Created during `registerPane`, marked `disposed` at the start of
 * `unregisterPane` (before either store deletes its slot). Hooks pass
 * `model.dispatchPane` / `model.dispatchDoc` so async call sites are
 * default-safe against post-unmount races without having to remember the
 * soft-dispatch variant per call site. The module-level `dispatchIfRegistered`
 * exports remain live for call sites that lack a model handle.
 */

import { trail } from "@/log/render-trail";
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
        // Crash-trace: every reducer command flows through here, so a
        // single trail point covers the whole agent-pane action surface.
        // The boundary dumps the trail when a render fault catches —
        // see frontend/log/render-trail.ts.
        trail("agent:dispatchPane", {
            block: this.blockId.slice(0, 7),
            cmd: command.type,
            source,
        });
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
