// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Type definitions for the launch-flow-state reducer.
 * Spec: docs/specs/SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md.
 *
 * Owns the entire editable Launch-modal surface as a single state
 * object:
 *   - `form` — name, runtime, image, identity/memory/continue selections
 *   - `identities` / `memories` — loaded bundle lists + load status
 *   - `bindings` — per-identity binding cache (push-updated via
 *     backend `identitybundlebindings:changed:<id>` events)
 *   - `submit` — submit-in-flight + last error
 *   - `auth` — folded-in OAuth state machine (Stage 2d) so the
 *     (auth × form-field-changed) cross-product is testable
 *     against a single pure reducer. The `Auth` command wraps
 *     `AuthCommand`s from auth-state.ts and delegates to that
 *     module's `update()`.
 *
 * Pure reducer — `update(state, command) → { state, events }`. The
 * view mounts the store, dispatches commands, and runs emitted
 * events (RPC calls etc.) outside the reducer.
 *
 * Reference: `frontend/app/store/browser-pane-state/types.ts` is the
 * established slice shape this file mirrors.
 */

import type { AuthCommand, AuthEvent, AuthState } from "@/app/view/agent/auth/auth-state";
import { initialState as initialAuthState } from "@/app/view/agent/auth/auth-state";

/** Editable Launch-modal form fields. Empty strings mean "no
 *  selection"; the view's submit predicate blocks Launch until the
 *  user picks or creates a real bundle. */
export interface LaunchForm {
    name: string;
    runtime: "host" | "container";
    image: string;
    /** Selected Identity bundle id. `""` = unselected. */
    identityId: string;
    /** Selected Memory bundle id. `""` = unselected. */
    memoryId: string;
    /** When set, this launch is a continuation of a prior named
     *  agent — pulled from the user's "Continue agent" dropdown.
     *  Drives the per-row lock semantics in §3.2.2 of the spec. */
    continueOfId: string | null;
}

export const initialForm = (): LaunchForm => ({
    name: "",
    runtime: "host",
    image: "",
    identityId: "",
    memoryId: "",
    continueOfId: null,
});

/** Wrapper for an asynchronous resource the reducer tracks. The
 *  view shows a loading state while `loading` is true; `error` is
 *  set on fetch failure and cleared on the next successful load. */
export interface ResourceList<T> {
    list: T[];
    loading: boolean;
    error: string | null;
}

export const initialResourceList = <T>(): ResourceList<T> => ({
    list: [],
    loading: false,
    error: null,
});

export interface SubmitStatus {
    inFlight: boolean;
    error: string | null;
}

export const initialSubmit = (): SubmitStatus => ({
    inFlight: false,
    error: null,
});

/** Top-level reducer state. */
export interface LaunchFlowState {
    form: LaunchForm;
    identities: ResourceList<IdentityBundle>;
    memories: ResourceList<Memory>;
    /** Per-identity binding cache. Keyed on `identityId`. Push-
     *  updated via `BindingsLoaded` (initial load) and `BindingsChanged`
     *  (subsequent backend events). Empty value means "no bindings yet
     *  / hasn't been queried"; the view distinguishes via the loading
     *  side-channel below. */
    bindings: Record<string, IdentityBinding[]>;
    /** Identity ids whose bindings query is currently in flight.
     *  Used by the view to render the auth gate as "loading" so a
     *  fast Connect click doesn't slip through the race. */
    bindingsLoading: Record<string, boolean>;
    submit: SubmitStatus;
    /** Folded-in OAuth state machine. The `Auth` command wraps an
     *  `AuthCommand` from auth-state.ts; the reducer delegates to
     *  that module's `update()` so this slice stays the single
     *  source of truth (Stage 2d). */
    auth: AuthState;
    /** Terminal flag — set by `Closed`. Post-close commands are no-ops. */
    closed: boolean;
}

export const initialState = (): LaunchFlowState => ({
    form: initialForm(),
    identities: initialResourceList<IdentityBundle>(),
    memories: initialResourceList<Memory>(),
    bindings: {},
    bindingsLoading: {},
    submit: initialSubmit(),
    auth: initialAuthState(),
    closed: false,
});

/** All transitions the view can dispatch. */
export type LaunchFlowCommand =
    /** Modal opened — initial form values (preselect / continuation
     *  carry-over from the picker). */
    | { type: "Opened"; initial?: Partial<LaunchForm> }
    /** Form field commands. Each is idempotent (set-to-current is a
     *  no-op the reducer's identity check filters out). */
    | { type: "NameChanged"; name: string }
    | { type: "RuntimeChanged"; runtime: "host" | "container" }
    | { type: "ImageChanged"; image: string }
    /** Setting identity to `""` clears it (e.g. on legacy
     *  continuation with no carry-over). The view auto-picks the
     *  first non-blank bundle elsewhere. */
    | { type: "IdentityChanged"; identityId: string }
    | { type: "MemoryChanged"; memoryId: string }
    /** Continue dropdown — `null` = "— New agent —". Setting to a
     *  real instance id locks per-row selectors with the carry-over
     *  values. */
    | {
          type: "ContinueOfChanged";
          continueOfId: string | null;
          carry?: { name: string; identityId: string; memoryId: string };
      }
    /** Resource lifecycle commands. */
    | { type: "IdentitiesLoading" }
    | { type: "IdentitiesLoaded"; list: IdentityBundle[] }
    | { type: "IdentitiesFailed"; error: string }
    | { type: "MemoriesLoading" }
    | { type: "MemoriesLoaded"; list: Memory[] }
    | { type: "MemoriesFailed"; error: string }
    /** Bindings push events. `BindingsChanged` is the wire shape the
     *  backend will emit in Stage 2c; until then it's only used by
     *  unit tests. */
    | { type: "BindingsLoading"; identityId: string }
    | { type: "BindingsLoaded"; identityId: string; bindings: IdentityBinding[] }
    | { type: "BindingsChanged"; identityId: string; bindings: IdentityBinding[] }
    /** Submit lifecycle. */
    | { type: "SubmitClicked" }
    | { type: "SubmitSucceeded" }
    | { type: "SubmitFailed"; error: string }
    /** Wrapped auth-state command. Reducer delegates to the auth
     *  module's pure `update()`; any auth events the inner reducer
     *  emits get wrapped as `AuthEvent` and surfaced on the outer
     *  ReducerResult. */
    | { type: "Auth"; cmd: AuthCommand }
    /** Terminal. */
    | { type: "Closed" };

/** Side-effects the view runs in response to commands. The reducer
 *  stays pure; emits these for the view to dispatch onto the wire.
 *  Each event is keyed/tagged so the view can dedupe in-flight RPCs. */
export type LaunchFlowEvent =
    /** Fetch bindings for an identity that just became selected and
     *  has no cached entry. */
    | { type: "FetchBindings"; identityId: string }
    /** Pass-through of an auth-state event so the view can run the
     *  side-effect (RPC, openExternal, etc.). Wraps `AuthEvent` from
     *  auth-state.ts. */
    | { type: "Auth"; event: AuthEvent };

export interface ReducerResult {
    state: LaunchFlowState;
    events: LaunchFlowEvent[];
}

// ── Selectors ───────────────────────────────────────────────────────────────
//
// Pure read-only derivations over state. The view calls these instead
// of repeating logic — keeps the cross-product testable in one place.

/** Real (non-blank) identity bundles, filtering out any
 *  back-compat `is_blank` singleton the backend may still return. */
export function realIdentities(state: LaunchFlowState): IdentityBundle[] {
    return state.identities.list.filter((b) => !b.is_blank);
}

/** Real (non-blank) memory bundles. */
export function realMemories(state: LaunchFlowState): Memory[] {
    return state.memories.list.filter((m) => !m.is_blank);
}

/** True when the form is a continuation of a prior named agent
 *  (Continue dropdown set to a real instance id). */
export function isContinue(state: LaunchFlowState): boolean {
    return state.form.continueOfId !== null;
}

/** Per-row continuation lock. Identity selector locks ONLY when
 *  the continued row carried a real (non-empty, non-legacy-"blank")
 *  identity_id. Legacy carry-overs leave the selector editable so
 *  the user can pick a replacement (codex P1 on PR #916). */
export function continueLocksIdentity(state: LaunchFlowState): boolean {
    return isContinue(state) && state.form.identityId !== "";
}

export function continueLocksMemory(state: LaunchFlowState): boolean {
    return isContinue(state) && state.form.memoryId !== "";
}

/** Whether the selected identity bundle has a binding for the given
 *  provider. Used by the auth-gate predicate. While bindings are
 *  loading, returns false (safe — the gate stays on). */
export function hasMatchingBinding(
    state: LaunchFlowState,
    providerId: string,
): boolean {
    const id = state.form.identityId;
    if (!id) return false;
    if (state.bindingsLoading[id]) return false;
    const bindings = state.bindings[id] ?? [];
    return bindings.some((b) => b.provider === providerId);
}

/** Whether Launch can fire. Combines form completeness with the
 *  caller-supplied auth-ready check (the auth state machine still
 *  lives in AuthFlowController for now; the view supplies its
 *  `authStateKind === "ready"` boolean). */
export function canSubmit(
    state: LaunchFlowState,
    opts: { authReady: boolean; nameValid: boolean },
): boolean {
    if (state.submit.inFlight) return false;
    if (!opts.nameValid) return false;
    if (state.form.identityId === "") return false;
    if (state.form.memoryId === "") return false;
    if (!opts.authReady) return false;
    return true;
}
