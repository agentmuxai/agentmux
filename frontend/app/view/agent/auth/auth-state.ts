// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pre-launch OAuth state machine — frontend side of the flow defined
 * in `docs/specs/SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md` §8.
 *
 * Pure reducer (`update(state, command) → { state, events }`) — no
 * RPC, no DOM. The `AgentLaunchModal` owns one instance per open;
 * effects (RPC calls, URL paste, etc.) are run by the modal in
 * response to emitted events.
 *
 * Why a reducer (not a tangle of `createSignal`s): the auth flow has
 * 5 transient states (idle/unauthenticated/waiting/ready/expired) and
 * 8 commands that mutate them. The combinations are non-trivial — a
 * poll result while the user is mid-cancel, a callback URL paste
 * while the session is already terminal, an API-key submit when the
 * dropdown swaps mid-flight. The reducer makes each transition a
 * single clearly-named case with idempotency rules, and the unit
 * tests pin every combo.
 *
 * Backend wire types are mirrored in `frontend/types/gotypes.d.ts`
 * (`AuthSessionStatus` etc.). This module imports those globals so
 * `Selected` / `Polled` carry the same shapes the RPC handlers
 * return.
 */

/** Mirrors the wire `AuthSessionStatus` from
 *  `agentmux-srv/src/identity/auth_session.rs` (camelCase via
 *  `rename_all_fields`). Pulled inline here as a TS type because the
 *  Rust enum doesn't currently surface in `gotypes.d.ts`. */
export type AuthSessionStatusWire =
    | { status: "pending" }
    | { status: "url-available"; authUrl: string }
    | { status: "code-emitted"; deviceCode: string; verificationUrl: string }
    | { status: "success"; bundleId: string; email: string | null }
    | { status: "failed"; error: string };

/** Outcome of the modal's bundle/provider selection. The view computes
 *  this from the dropdown + the loaded `IdentityBundle` / `Memory`
 *  lists and dispatches `Selected` once. */
export type SelectionOutcome =
    | "ready" // bundle has authenticated account for this provider
    | "expired" // bundle has account but stale (re-auth needed)
    | "needs-account" // bundle exists but no account for this provider
    | "needs-bundle"; // blank singleton — Connect creates new bundle

export interface AuthState {
    /** Terminal flag set by `Disposed`. Post-close commands are no-ops. */
    closed: boolean;
    /** What the view should render. The `unauthenticated` and
     *  `expired` kinds drive different copy in the inline CTA. */
    kind: "idle" | "unauthenticated" | "waiting" | "ready" | "expired" | "failed";
    /** Selected provider id ("claude", "codex", "openclaw", ...). */
    providerId: string;
    /** Selected identity bundle id. Empty when no bundle picked OR
     *  blank singleton (`needs-bundle`). */
    bundleId: string;
    /** Active backend session id when `kind === "waiting"`. */
    sessionId: string;
    /** Captured OAuth URL — surface in the inline panel if the
     *  browser didn't open. */
    authUrl: string;
    /** Captured device-flow code (Copilot path). */
    deviceCode: { code: string; verificationUrl: string } | null;
    /** Last error message — populated on `Failed` so the inline
     *  banner can render it. Cleared by the next selection or
     *  connect attempt. */
    error: string;
}

export const initialState = (): AuthState => ({
    closed: false,
    kind: "idle",
    providerId: "",
    bundleId: "",
    sessionId: "",
    authUrl: "",
    deviceCode: null,
    error: "",
});

export type AuthCommand =
    /**
     * The view's dropdown selection settled. The view computes which
     * `outcome` applies from the bundle data it just loaded and
     * passes it in — the reducer doesn't need to re-derive.
     */
    | {
          type: "Selected";
          providerId: string;
          bundleId: string;
          outcome: SelectionOutcome;
      }
    /**
     * The user clicked "Connect with OAuth". The view will fire the
     * `StartProviderAuth` RPC on the corresponding event; this
     * command just transitions to `waiting`.
     */
    | { type: "ConnectClicked" }
    /**
     * RPC returned with a session id (and possibly an immediate
     * URL — claude tends to print the URL within the first frame
     * of stdout).
     */
    | { type: "SessionStarted"; sessionId: string; authUrl?: string }
    /**
     * Poll returned a non-terminal status. The reducer extracts
     * the relevant fields. Multiple polls returning the same data
     * are idempotent (no event re-fired).
     */
    | { type: "Polled"; status: AuthSessionStatusWire }
    /**
     * The user clicked "Cancel login". The view fires
     * `CancelProviderAuth` on the corresponding event.
     */
    | { type: "CancelClicked" }
    /**
     * The user pasted a callback URL into the URL panel. View fires
     * `SubmitAuthCallback`. No state change here — we wait for the
     * next poll to see "success" or "failed".
     */
    | { type: "CallbackSubmitted"; callbackUrl: string }
    /**
     * API-key path: user pasted a key into the modal. View fires
     * `SubmitProviderApiKey`; reducer transitions to a brief
     * `waiting` state with no session id (the RPC is synchronous).
     */
    | { type: "ApiKeySubmitted"; apiKey: string; accountName: string }
    /**
     * Backend confirmed an API-key bundle creation. Mirrors
     * `Polled { status: "success" }` for the API-key path.
     */
    | { type: "ApiKeyAccepted"; bundleId: string }
    /**
     * Modal close / unmount. Idempotent.
     */
    | { type: "Disposed" };

export type AuthEvent =
    | {
          type: "selection-changed";
          providerId: string;
          bundleId: string;
          kind: AuthState["kind"];
      }
    | { type: "start-requested"; providerId: string; bundleId: string }
    | { type: "session-started"; sessionId: string; authUrl: string }
    | { type: "url-available"; authUrl: string }
    | {
          type: "device-code-emitted";
          code: string;
          verificationUrl: string;
      }
    | { type: "succeeded"; bundleId: string; email: string | null }
    | { type: "failed"; error: string }
    | { type: "cancel-requested"; sessionId: string }
    | { type: "callback-submit-requested"; sessionId: string; callbackUrl: string }
    | {
          type: "api-key-submit-requested";
          providerId: string;
          bundleId: string;
          apiKey: string;
          accountName: string;
      }
    | { type: "api-key-accepted"; bundleId: string }
    | { type: "disposed" }
    | { type: "post-close-command-dropped"; commandType: string };

export interface ReducerResult {
    state: AuthState;
    events: AuthEvent[];
}

const selectionKind = (outcome: SelectionOutcome): AuthState["kind"] => {
    switch (outcome) {
        case "ready":
            return "ready";
        case "expired":
            return "expired";
        case "needs-account":
        case "needs-bundle":
            return "unauthenticated";
    }
};

export function update(state: AuthState, command: AuthCommand): ReducerResult {
    if (state.closed && command.type !== "Disposed") {
        return {
            state,
            events: [
                { type: "post-close-command-dropped", commandType: command.type },
            ],
        };
    }

    switch (command.type) {
        case "Selected": {
            const nextKind = selectionKind(command.outcome);
            const next: AuthState = {
                ...state,
                providerId: command.providerId,
                bundleId: command.bundleId,
                kind: nextKind,
                sessionId: "",
                authUrl: "",
                deviceCode: null,
                error: "",
            };
            // Idempotency: same provider+bundle+outcome → no event.
            if (
                state.providerId === next.providerId &&
                state.bundleId === next.bundleId &&
                state.kind === nextKind
            ) {
                return { state, events: [] };
            }
            return {
                state: next,
                events: [
                    {
                        type: "selection-changed",
                        providerId: next.providerId,
                        bundleId: next.bundleId,
                        kind: nextKind,
                    },
                ],
            };
        }

        case "ConnectClicked": {
            if (state.kind !== "unauthenticated" && state.kind !== "expired" && state.kind !== "failed") {
                // No-op — can't start auth from `ready` / `waiting` /
                // `idle`. Surface it via the dropped event so a misfire
                // shows up in the audit ring.
                return {
                    state,
                    events: [
                        {
                            type: "post-close-command-dropped",
                            commandType: "ConnectClicked",
                        },
                    ],
                };
            }
            return {
                state: { ...state, kind: "waiting", error: "" },
                events: [
                    {
                        type: "start-requested",
                        providerId: state.providerId,
                        bundleId: state.bundleId,
                    },
                ],
            };
        }

        case "SessionStarted": {
            const authUrl = command.authUrl ?? "";
            return {
                state: {
                    ...state,
                    sessionId: command.sessionId,
                    authUrl,
                    kind: "waiting",
                },
                events: [
                    {
                        type: "session-started",
                        sessionId: command.sessionId,
                        authUrl,
                    },
                ],
            };
        }

        case "Polled": {
            return foldPolled(state, command.status);
        }

        case "CancelClicked": {
            if (state.kind !== "waiting" || state.sessionId === "") {
                return {
                    state,
                    events: [
                        {
                            type: "post-close-command-dropped",
                            commandType: "CancelClicked",
                        },
                    ],
                };
            }
            return {
                state: {
                    ...state,
                    kind: "unauthenticated",
                    sessionId: "",
                    authUrl: "",
                    deviceCode: null,
                },
                events: [{ type: "cancel-requested", sessionId: state.sessionId }],
            };
        }

        case "CallbackSubmitted": {
            if (state.kind !== "waiting" || state.sessionId === "") {
                return {
                    state,
                    events: [
                        {
                            type: "post-close-command-dropped",
                            commandType: "CallbackSubmitted",
                        },
                    ],
                };
            }
            return {
                state,
                events: [
                    {
                        type: "callback-submit-requested",
                        sessionId: state.sessionId,
                        callbackUrl: command.callbackUrl,
                    },
                ],
            };
        }

        case "ApiKeySubmitted": {
            return {
                state: { ...state, kind: "waiting", error: "" },
                events: [
                    {
                        type: "api-key-submit-requested",
                        providerId: state.providerId,
                        bundleId: state.bundleId,
                        apiKey: command.apiKey,
                        accountName: command.accountName,
                    },
                ],
            };
        }

        case "ApiKeyAccepted": {
            return {
                state: {
                    ...state,
                    kind: "ready",
                    bundleId: command.bundleId,
                    sessionId: "",
                    authUrl: "",
                    deviceCode: null,
                    error: "",
                },
                events: [{ type: "api-key-accepted", bundleId: command.bundleId }],
            };
        }

        case "Disposed": {
            if (state.closed) return { state, events: [] };
            return {
                state: { ...state, closed: true },
                events: [{ type: "disposed" }],
            };
        }
    }
}

function foldPolled(state: AuthState, status: AuthSessionStatusWire): ReducerResult {
    switch (status.status) {
        case "pending": {
            // No state change — keep `waiting`. Pure tick; the modal's
            // poll loop is what's interesting, not this transition.
            return { state, events: [] };
        }
        case "url-available": {
            if (state.authUrl === status.authUrl) return { state, events: [] };
            return {
                state: { ...state, authUrl: status.authUrl },
                events: [{ type: "url-available", authUrl: status.authUrl }],
            };
        }
        case "code-emitted": {
            const prior = state.deviceCode;
            if (
                prior &&
                prior.code === status.deviceCode &&
                prior.verificationUrl === status.verificationUrl
            ) {
                return { state, events: [] };
            }
            return {
                state: {
                    ...state,
                    deviceCode: {
                        code: status.deviceCode,
                        verificationUrl: status.verificationUrl,
                    },
                },
                events: [
                    {
                        type: "device-code-emitted",
                        code: status.deviceCode,
                        verificationUrl: status.verificationUrl,
                    },
                ],
            };
        }
        case "success": {
            return {
                state: {
                    ...state,
                    kind: "ready",
                    bundleId: status.bundleId,
                    sessionId: "",
                    authUrl: "",
                    deviceCode: null,
                    error: "",
                },
                events: [
                    {
                        type: "succeeded",
                        bundleId: status.bundleId,
                        email: status.email,
                    },
                ],
            };
        }
        case "failed": {
            return {
                state: {
                    ...state,
                    kind: "failed",
                    sessionId: "",
                    deviceCode: null,
                    error: status.error,
                },
                events: [{ type: "failed", error: status.error }],
            };
        }
    }
}
