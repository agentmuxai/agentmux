// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for the launch-flow-state slice. Mirrors the
 * `update(state, command) → { state, events }` shape used by
 * `frontend/app/store/browser-pane-state/reducer.ts`.
 *
 * Stage 2a — additive. The view migration (AgentLaunchModal swap to
 * `useLaunchFlowStore()`) is Stage 2b. See spec
 * `docs/specs/SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md`.
 */

import type {
    LaunchFlowCommand,
    LaunchFlowEvent,
    LaunchFlowState,
    ReducerResult,
} from "./types";
import { initialForm } from "./types";

/** Helper: emit `FetchBindings` exactly when an identityId is real
 *  (non-empty), not yet cached, and not already in flight. Three
 *  commands trigger this fetch (Opened with preselect,
 *  IdentityChanged, ContinueOfChanged with carry-over), so the
 *  predicate lives here to keep the rule in one place. */
function maybeFetchBindings(
    state: LaunchFlowState,
    identityId: string,
): LaunchFlowEvent[] {
    if (identityId === "") return [];
    if (state.bindings[identityId] !== undefined) return [];
    if (state.bindingsLoading[identityId]) return [];
    return [{ type: "FetchBindings", identityId }];
}

export function update(
    state: LaunchFlowState,
    command: LaunchFlowCommand,
): ReducerResult {
    // Closed terminal — every command after Closed is a no-op so
    // late events from a torn-down modal don't poke the state.
    // `Opened` is the explicit re-arm path (reopen the modal) and
    // bypasses the guard.
    if (state.closed && command.type !== "Closed" && command.type !== "Opened") {
        return { state, events: [] };
    }

    switch (command.type) {
        case "Opened": {
            const next: LaunchFlowState = {
                ...state,
                form: { ...initialForm(), ...command.initial },
                closed: false,
            };
            return { state: next, events: maybeFetchBindings(state, next.form.identityId) };
        }

        case "NameChanged": {
            if (state.form.name === command.name) return { state, events: [] };
            return {
                state: { ...state, form: { ...state.form, name: command.name } },
                events: [],
            };
        }

        case "RuntimeChanged": {
            if (state.form.runtime === command.runtime) return { state, events: [] };
            return {
                state: { ...state, form: { ...state.form, runtime: command.runtime } },
                events: [],
            };
        }

        case "ImageChanged": {
            if (state.form.image === command.image) return { state, events: [] };
            return {
                state: { ...state, form: { ...state.form, image: command.image } },
                events: [],
            };
        }

        case "IdentityChanged": {
            if (state.form.identityId === command.identityId) {
                return { state, events: [] };
            }
            return {
                state: { ...state, form: { ...state.form, identityId: command.identityId } },
                events: maybeFetchBindings(state, command.identityId),
            };
        }

        case "MemoryChanged": {
            if (state.form.memoryId === command.memoryId) return { state, events: [] };
            return {
                state: { ...state, form: { ...state.form, memoryId: command.memoryId } },
                events: [],
            };
        }

        case "ContinueOfChanged": {
            // Idempotency: re-dispatching with the same continueOfId
            // is a no-op so a stray repeat from the view doesn't
            // overwrite mid-form edits with the original carry-over.
            // Compare on continueOfId; the carry values are a
            // deterministic projection of the row, so if the id is
            // unchanged the carry is too.
            if (state.form.continueOfId === command.continueOfId) {
                return { state, events: [] };
            }
            const next: LaunchFlowState = {
                ...state,
                form: {
                    ...state.form,
                    continueOfId: command.continueOfId,
                    // Carry-over (or clear) the form fields when the
                    // user picks a row. `carry` is supplied by the
                    // caller because the row's instance_name /
                    // identity_id / memory_id translation (legacy
                    // "" or "blank" → "") is a view concern.
                    name: command.carry?.name ?? "",
                    identityId: command.carry?.identityId ?? "",
                    memoryId: command.carry?.memoryId ?? "",
                },
            };
            return {
                state: next,
                events: maybeFetchBindings(state, next.form.identityId),
            };
        }

        case "IdentitiesLoading": {
            return {
                state: {
                    ...state,
                    identities: { ...state.identities, loading: true, error: null },
                },
                events: [],
            };
        }

        case "IdentitiesLoaded": {
            return {
                state: {
                    ...state,
                    identities: { list: command.list, loading: false, error: null },
                },
                events: [],
            };
        }

        case "IdentitiesFailed": {
            return {
                state: {
                    ...state,
                    identities: { ...state.identities, loading: false, error: command.error },
                },
                events: [],
            };
        }

        case "MemoriesLoading": {
            return {
                state: { ...state, memories: { ...state.memories, loading: true, error: null } },
                events: [],
            };
        }

        case "MemoriesLoaded": {
            return {
                state: { ...state, memories: { list: command.list, loading: false, error: null } },
                events: [],
            };
        }

        case "MemoriesFailed": {
            return {
                state: { ...state, memories: { ...state.memories, loading: false, error: command.error } },
                events: [],
            };
        }

        case "BindingsLoading": {
            return {
                state: {
                    ...state,
                    bindingsLoading: { ...state.bindingsLoading, [command.identityId]: true },
                },
                events: [],
            };
        }

        case "BindingsLoaded":
        case "BindingsChanged": {
            // Both commands write the new bindings array. They differ
            // only in what they do to the per-id loading flag:
            //   - BindingsLoaded settles a fetch the reducer asked
            //     for via FetchBindings → clears the loading flag.
            //   - BindingsChanged is a backend push event arriving
            //     unsolicited (no in-flight fetch) → leaves the
            //     loading flag alone (likely already absent).
            const nextLoading = { ...state.bindingsLoading };
            if (command.type === "BindingsLoaded") {
                delete nextLoading[command.identityId];
            }
            return {
                state: {
                    ...state,
                    bindings: { ...state.bindings, [command.identityId]: command.bindings },
                    bindingsLoading: nextLoading,
                },
                events: [],
            };
        }

        case "SubmitClicked": {
            // Idempotent under in-flight — second click is a no-op.
            if (state.submit.inFlight) return { state, events: [] };
            return {
                state: { ...state, submit: { inFlight: true, error: null } },
                events: [],
            };
        }

        case "SubmitSucceeded": {
            return {
                state: { ...state, submit: { inFlight: false, error: null } },
                events: [],
            };
        }

        case "SubmitFailed": {
            return {
                state: { ...state, submit: { inFlight: false, error: command.error } },
                events: [],
            };
        }

        case "Closed": {
            if (state.closed) return { state, events: [] };
            return { state: { ...state, closed: true }, events: [] };
        }
    }
}
