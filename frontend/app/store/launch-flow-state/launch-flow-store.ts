// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Solid binding for the launch-flow-state reducer. Owns a per-modal
 * reactive store + a `dispatch(cmd)` that runs the pure reducer and
 * forwards emitted events to a caller-installed handler.
 *
 * Single-instance per modal (unlike browser-pane-state-store which
 * is keyed-by-block) so the surface is much smaller — no slot map,
 * no register/unregister, just `createLaunchFlowStore()` returning
 * `{ state, dispatch }`.
 *
 * Every dispatch is recorded into the global audit ring (§6.8 of
 * SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md) so the diag panel
 * + tests can replay transitions. The ring stores command + emitted
 * events; state diffs are reconstructable by replaying.
 */

import { createStore, reconcile, unwrap, type Store } from "solid-js/store";

import { type CommandSource, recordDispatch } from "../command-source";
import { update } from "./reducer";
import { initialState, type LaunchFlowCommand, type LaunchFlowEvent, type LaunchFlowState } from "./types";

export interface LaunchFlowStore {
    /** Reactive view into the current state. `state.form.name` etc.
     *  track at the leaf — only reads that touch a changed leaf
     *  re-fire dependent effects. */
    state: Store<LaunchFlowState>;
    /** Run a command through the reducer. Diffs old→new via Solid's
     *  `reconcile` so unchanged subtrees keep the same proxy
     *  identity (no spurious re-renders for fields the command
     *  didn't touch). Emitted events are forwarded to `eventSink`.
     *  Optional `source` tags the recordDispatch entry; defaults to
     *  `"user"` since most dispatches come from form interactions.
     *  System-driven dispatches (initial resource loads, event-sink
     *  followups) should pass `"system"`. */
    dispatch: (cmd: LaunchFlowCommand, source?: CommandSource) => void;
}

export interface LaunchFlowStoreOptions {
    /** Run side-effects emitted by the reducer (RPC calls, etc.). */
    eventSink?: (event: LaunchFlowEvent) => void;
}

export function createLaunchFlowStore(
    opts: LaunchFlowStoreOptions = {},
): LaunchFlowStore {
    const [state, setState] = createStore<LaunchFlowState>(initialState());
    const sink = opts.eventSink ?? (() => {});
    const dispatch = (cmd: LaunchFlowCommand, source: CommandSource = "user"): void => {
        const snapshot = unwrap(state);
        const result = update(snapshot, cmd);
        if (result.state !== snapshot) {
            setState(reconcile(result.state));
        }
        recordDispatch({
            slice: "launch-flow-state",
            key: null,
            command: cmd,
            events: result.events,
            source,
            at: Date.now(),
        });
        for (const event of result.events) {
            sink(event);
        }
    };
    return { state, dispatch };
}
