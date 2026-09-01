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
 *  `rename_all_fields`).
 *
 *  Intentionally duplicates the global `AuthSessionStatus` type in
 *  `frontend/types/gotypes.d.ts` (added in PR #850). The duplication
 *  is so that the reducer's command surface is self-contained —
 *  `auth-state.ts` declares its own types without depending on the
 *  ambient global `declare`, which makes the file importable and
 *  pure-testable. The two shapes must stay in lockstep.
 *
 *  Two terminal-ish variants:
 *  - `authenticated`: CLI auth confirmed but no bundle row exists yet.
 *    User chooses a name → frontend fires `auth.savebundle` → backend
 *    transitions to `success` with the real bundleId.
 *  - `success`: post-save final state, bundleId is the real row id.
 *
 *  `accountId` (issue #1624 PR-C Part B) is set only for a direct-
 *  account session (no bundle involved) — `bundleId` is `""` in that
 *  mode. Wire-additive only in this PR; not yet consumed by the
 *  reducer below (that's PR 3 of
 *  docs/specs/SPEC_IDENTITY_DIRECT_LINKS_PHASE3_PRC_2026_07_10.md).
 */
export type AuthSessionStatusWire =
    | { status: "pending" }
    | { status: "url-available"; authUrl: string }
    | { status: "code-emitted"; deviceCode: string; verificationUrl: string }
    | { status: "authenticated"; email: string | null }
    | { status: "success"; bundleId: string; email: string | null; accountId?: string }
    | { status: "failed"; error: string };

/** Outcome of the modal's account/provider selection. The view computes
 *  this from the dropdown + the loaded account list and dispatches
 *  `Selected` once.
 *
 *  Issue #1624 PR-C Part B dropped `"needs-bundle"` (blank-singleton —
 *  Connect creates a new bundle first): the launch modal no longer has
 *  a bundle picker to be blank, so that outcome can never be produced
 *  by any caller. `selectionKind()` already treated it identically to
 *  `"needs-account"`, so removing it is a pure type-level prune, no
 *  reducer behavior change. */
export type SelectionOutcome =
    | "ready" // account is authenticated for this provider
    | "expired" // account exists but stale (re-auth needed)
    | "needs-account"; // no account selected (or selected account doesn't match this provider)

export interface AuthState {
    /** Terminal flag set by `Disposed`. Post-close commands are no-ops. */
    closed: boolean;
    /**
     * What the view should render.
     * - `idle`: pre-selection, view has not dispatched `Selected` yet.
     * - `unauthenticated`: Connect CTA visible. Launch disabled.
     * - `waiting`: OAuth/api-key RPC in flight, CLI running. Cancel button.
     * - `authenticated`: CLI auth done, awaiting `SaveBundleClicked`. SaveBundle panel.
     * - `saving`: `auth.savebundle` RPC in flight. Spinner.
     * - `ready`: bundle row exists in DB. Launch enabled.
     * - `expired`: bundle has provider account but creds stale. "Re-authenticate" CTA.
     * - `failed`: terminal failure. FailedBanner with Retry.
     *
     * See `docs/specs/SPEC_LAUNCH_AUTH_STATE_MACHINE_2026_05_14.md` §4.
     */
    kind:
        | "idle"
        | "unauthenticated"
        | "waiting"
        | "authenticated"
        | "saving"
        | "ready"
        | "expired"
        | "failed";
    /** Selected provider id ("claude", "codex", "openclaw", ...). */
    providerId: string;
    /** Selected identity bundle id. Empty when no bundle picked OR
     *  blank singleton (`needs-bundle`). */
    bundleId: string;
    /** When the connect is a "re-auth" or "add account" flow, the
     *  existing bundle id to update on save. Empty = create new bundle. */
    intoBundleId: string;
    /** Active backend session id when `kind === "waiting" | "authenticated" | "saving"`. */
    sessionId: string;
    /** Captured OAuth URL — surface in the inline panel if the
     *  browser didn't open. */
    authUrl: string;
    /** Captured device-flow code (Copilot path). */
    deviceCode: { code: string; verificationUrl: string } | null;
    /** Email captured from the auth-success line (OAuth) or accountName
     *  (API-key). The view uses this to prefill the SaveBundle name input. */
    email: string;
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
    intoBundleId: "",
    sessionId: "",
    authUrl: "",
    deviceCode: null,
    email: "",
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
     * Poll returned a status for the session identified by
     * `sessionId`. The reducer drops the result if it doesn't match
     * the currently-active session — guards against stale polls from
     * a cancelled or superseded session that would otherwise flip
     * state to a wrong `bundleId` (codex P1 on PR #845). Multiple
     * polls returning the same data are idempotent (no event re-fired).
     */
    | { type: "Polled"; sessionId: string; status: AuthSessionStatusWire }
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
     * Backend confirmed an API-key bundle creation. API-key flow is
     * SINGLE-phase (backend persists the bundle in `auth.submitapikey`
     * itself) — distinct from the 2-phase OAuth path. Codex P1/P2 on
     * #853 + reagent on #847: the spec's 2-phase api-key flow needs a
     * backend `auth.savebundle` that doesn't exist yet (PR C-2). Until
     * then we keep the existing single-phase: validate-and-persist in
     * one RPC, transition straight to `ready`.
     */
    | { type: "ApiKeyAccepted"; bundleId: string }
    /**
     * A completed login's account is registered, so auth is satisfied and the
     * reducer may transition to `ready`.
     *
     * NAME IS HISTORICAL: until 2026-08-31 this meant "seed from global" —
     * the user's personal `~/.claude` credential copied into the agent's
     * isolated dir with no OAuth at all. That tier was a per-channel-isolation
     * bypass and was removed
     * (docs/analysis/ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md #3);
     * the action name is kept to avoid churning this union and its tests.
     * Formerly offered as tier 2
     * alongside the in-app session (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md
     * §3.1) that's since become tier 1's primary path for Claude v2.1.198+.
     * Single-phase, like `ApiKeyAccepted`: the credential file IS the
     * persistence, so transition straight to `ready`. Honored only from
     * connect-able kinds (`unauthenticated`/`expired`/`failed`); a stale
     * dispatch from `ready`/`waiting`/etc. is dropped. `bundleId` may be ""
     * for the default identity (the existing dir is seeded in place — no new
     * bundle row created).
     */
    | { type: "Seeded"; bundleId: string }
    /**
     * User confirmed the bundle name in the SaveBundle panel. View
     * fires `auth.savebundle` RPC on the emitted event. Transitions
     * to `saving`. Reagent-pinned: only honored from `authenticated`.
     */
    | { type: "SaveBundleClicked"; name: string }
    /**
     * `auth.savebundle` RPC returned successfully. The bundle row +
     * account row + binding row are persisted. Transitions to
     * `ready` with the real bundleId. Only honored from `saving`.
     */
    | { type: "BundleSaved"; bundleId: string }
    /**
     * `auth.savebundle` RPC failed (e.g. UNIQUE constraint on bundle
     * name, transaction error). Transitions back to `authenticated`
     * (preserves email / sessionId so the user can edit the name and
     * retry). Only honored from `saving`.
     */
    | { type: "BundleSaveFailed"; error: string }
    /**
     * View-side prep failure (e.g. `ResolveCli` threw before
     * `auth.start` could fire, network preflight, etc.). The
     * controller's `failConnect` dispatches this to surface the
     * error inline. Honored only from connect-attempt kinds
     * (`unauthenticated`/`waiting`/`expired`/`failed`) — codex P2
     * on #853 round 7: stale rejections from abandoned connects
     * must NOT clobber a newer `ready`/`authenticated`/`saving`/
     * `idle` state.
     */
    | { type: "ConnectFailed"; error: string }
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
    /** CLI authenticated; SaveBundle panel should appear. Email is the
     *  account identifier captured from the provider (best-effort). */
    | { type: "authenticated"; email: string }
    /** User clicked Save with `name`; controller should fire
     *  `auth.savebundle` RPC against `sessionId`. */
    | {
          type: "save-bundle-requested";
          sessionId: string;
          intoBundleId: string;
          name: string;
      }
    /** Save RPC succeeded — bundle id now exists in `db_identity_bundles`. */
    | { type: "bundle-saved"; bundleId: string }
    /** Save RPC failed — name conflict, DB error, etc. */
    | { type: "bundle-save-failed"; error: string }
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
    | { type: "seeded"; bundleId: string }
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
            // Derive intoBundleId from outcome: re-auth (`expired`) and
            // add-account (`needs-account`) flows save into the existing
            // bundle row; `needs-bundle` (blank) creates new.
            const intoBundleId =
                command.outcome === "expired" || command.outcome === "needs-account"
                    ? command.bundleId
                    : "";
            const next: AuthState = {
                ...state,
                providerId: command.providerId,
                bundleId: command.bundleId,
                kind: nextKind,
                intoBundleId,
                sessionId: "",
                authUrl: "",
                deviceCode: null,
                email: "",
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
            // reagent P0 on #2262: `ready` is now accepted too — the
            // Reconnect CTA (PreLaunchAuthPanel.tsx's requiresLoginTty
            // branch) dispatches ConnectClicked from EXACTLY this state
            // (stale needs_reauth/expired account, still `ready` per
            // outcomeFor()'s "needs-account"/"ready" split). Without this,
            // the dispatch was a silent no-op: state never left `ready`, so
            // runProviderLogin ran the real backend login/refresh, but
            // every one of its outcome dispatches (Seeded, ConnectFailed)
            // was ALSO dropped from `ready` — the UI just sat on the same
            // Reconnect CTA forever regardless of success or failure. No
            // other caller dispatches ConnectClicked from a plain `ready`
            // (a healthy account renders `<ReadyBanner/>` with no Connect
            // affordance at all), so this widening is scoped to exactly
            // the reconnect flow it's meant for. `Seeded`/`ConnectFailed`
            // already accept `waiting` as an origin (see their own doc
            // comments) — this is the missing first hop into `waiting`
            // that lets those existing guards actually fire.
            if (
                state.kind !== "unauthenticated" &&
                state.kind !== "expired" &&
                state.kind !== "failed" &&
                state.kind !== "ready"
            ) {
                // No-op — can't start auth from `waiting` / `idle` /
                // `authenticated` / `saving`. Surface it via the dropped
                // event so a misfire shows up in the audit ring.
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
            // Reagent P1 on #849: only honor SessionStarted while
            // the reducer is still in `waiting`. If the user clicked
            // Cancel during the auth.start RPC await, kind has moved
            // to `unauthenticated` and sessionId is "" — a late
            // SessionStarted would otherwise create a zombie session
            // (kind back to "waiting" with a fresh sessionId) that
            // CancelClicked can no longer clear because the controller
            // already invoked cancel on the previous (cleared)
            // sessionId.
            if (state.kind !== "waiting") {
                return {
                    state,
                    events: [
                        {
                            type: "post-close-command-dropped",
                            commandType: "SessionStarted",
                        },
                    ],
                };
            }
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
            // Drop stale polls from cancelled / superseded sessions
            // (codex P1 on PR #845). The view passes the sessionId
            // the RPC was issued against; if it doesn't match the
            // currently-active session the result is from an old
            // attempt that the user already left behind.
            if (
                state.kind !== "waiting" ||
                state.sessionId === "" ||
                command.sessionId !== state.sessionId
            ) {
                return {
                    state,
                    events: [
                        {
                            type: "post-close-command-dropped",
                            commandType: "Polled",
                        },
                    ],
                };
            }
            return foldPolled(state, command.status);
        }

        case "CancelClicked": {
            // PR C-1 (S21): also allow cancel from `authenticated` —
            // user changed their mind in the SaveBundle panel; backend
            // session is still alive until savebundle commits, so we
            // need to tell the backend to drop it.
            //
            // Reagent P1 on #853 round 10: also allow cancel from
            // `saving` — the backend session is held alive for the
            // savebundle RPC; without this guard the controller's
            // CancelClicked dispatch is silently dropped and the
            // SaveBundle spinner is stuck forever.
            //
            // Reagent P1 on #850: allow cancel from `waiting` even when
            // sessionId === "" — that's the startup window between
            // ConnectClicked and SessionStarted (auth.start in flight).
            // The controller bumps actionToken so the pending start's
            // stale-token gate fires and the orphan SessionStarted is
            // dropped. User's cancel intent always wins.
            if (state.kind !== "waiting" && state.kind !== "authenticated" &&
                state.kind !== "saving") {
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
                    email: "",
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
            // Reagent P1 on PR #845: ApiKeySubmitted previously
            // spread `...state` into `waiting`, preserving any
            // stale OAuth `sessionId`. A late OAuth poll `success`
            // could then match (sessionId !== "") and flip state
            // to `ready` with the WRONG bundle. Drop the OAuth
            // session id + URL + device-code so this attempt is
            // unambiguously the API-key path.
            if (state.kind !== "unauthenticated" && state.kind !== "expired" && state.kind !== "failed") {
                return {
                    state,
                    events: [
                        {
                            type: "post-close-command-dropped",
                            commandType: "ApiKeySubmitted",
                        },
                    ],
                };
            }
            return {
                state: {
                    ...state,
                    kind: "waiting",
                    sessionId: "",
                    authUrl: "",
                    deviceCode: null,
                    error: "",
                },
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
            // Reagent P2 on #849: only honor ApiKeyAccepted while
            // the reducer is still in `waiting` for this submit. If
            // the user picked a different bundle or cancelled during
            // the API-key RPC await, kind has left `waiting` — a late
            // accept would otherwise flip state forward with stale data.
            if (state.kind !== "waiting") {
                return {
                    state,
                    events: [
                        {
                            type: "post-close-command-dropped",
                            commandType: "ApiKeyAccepted",
                        },
                    ],
                };
            }
            // PR C-1 (revised): api-key stays SINGLE-phase until C-2
            // backend `auth.savebundle` lands. Backend persists the
            // bundle in `auth.submitapikey`; we go straight to `ready`
            // with the real bundleId.
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

        case "Seeded": {
            // Honored from connect-able kinds AND `waiting` — the latter
            // covers `runProviderLogin`-driven connects (PreLaunchAuthPanel's
            // requiresLoginTty path), which dispatch ConnectClicked (→
            // `waiting`) up front so the panel shows progress during tier 3's
            // up-to-5-minute terminal wait, then `Seeded` on success. The
            // original single-phase "Use my existing login" caller (removed
            // 2026-08-31) never
            // enters `waiting` at all, so this widening doesn't change its
            // behavior. Still guards against a stale dispatch clobbering a
            // newer `ready`/`saving`/`idle`.
            if (
                state.kind !== "unauthenticated" &&
                state.kind !== "expired" &&
                state.kind !== "failed" &&
                state.kind !== "waiting"
            ) {
                return {
                    state,
                    events: [
                        { type: "post-close-command-dropped", commandType: "Seeded" },
                    ],
                };
            }
            // Single-phase (the seeded credential file IS the persistence) →
            // straight to `ready`. bundleId may be "" for the default identity
            // (no new bundle row — the existing isolated dir was seeded in place).
            const bundleId = command.bundleId || state.bundleId;
            return {
                state: {
                    ...state,
                    kind: "ready",
                    bundleId,
                    sessionId: "",
                    authUrl: "",
                    deviceCode: null,
                    error: "",
                },
                events: [{ type: "seeded", bundleId }],
            };
        }

        case "SaveBundleClicked": {
            if (state.kind !== "authenticated") {
                return {
                    state,
                    events: [
                        {
                            type: "post-close-command-dropped",
                            commandType: "SaveBundleClicked",
                        },
                    ],
                };
            }
            return {
                state: { ...state, kind: "saving", error: "" },
                events: [
                    {
                        type: "save-bundle-requested",
                        sessionId: state.sessionId,
                        intoBundleId: state.intoBundleId,
                        name: command.name,
                    },
                ],
            };
        }

        case "BundleSaved": {
            if (state.kind !== "saving") {
                return {
                    state,
                    events: [
                        {
                            type: "post-close-command-dropped",
                            commandType: "BundleSaved",
                        },
                    ],
                };
            }
            return {
                state: {
                    ...state,
                    kind: "ready",
                    bundleId: command.bundleId,
                    sessionId: "",
                    error: "",
                },
                events: [{ type: "bundle-saved", bundleId: command.bundleId }],
            };
        }

        case "BundleSaveFailed": {
            if (state.kind !== "saving") {
                return {
                    state,
                    events: [
                        {
                            type: "post-close-command-dropped",
                            commandType: "BundleSaveFailed",
                        },
                    ],
                };
            }
            // Return to `authenticated` so the user can edit the name
            // (e.g. resolve a name-collision) and retry. Keep sessionId
            // + email so the next SaveBundleClicked can fire savebundle
            // against the same backend session.
            return {
                state: { ...state, kind: "authenticated", error: command.error },
                events: [{ type: "bundle-save-failed", error: command.error }],
            };
        }

        case "ConnectFailed": {
            // Codex P2 on #853 round 7: gate on connect-attempt kinds
            // so a stale ResolveCli/ensureAuthDir rejection from an
            // abandoned connect can't clobber a newer `ready`/
            // `authenticated`/`saving` selection. Only honored where
            // a connect was actually in progress.
            if (state.kind !== "waiting" && state.kind !== "unauthenticated" &&
                state.kind !== "expired" && state.kind !== "failed") {
                return {
                    state,
                    events: [
                        {
                            type: "post-close-command-dropped",
                            commandType: "ConnectFailed",
                        },
                    ],
                };
            }
            return {
                state: {
                    ...state,
                    kind: "failed",
                    sessionId: "",
                    authUrl: "",
                    deviceCode: null,
                    error: command.error,
                },
                events: [{ type: "failed", error: command.error }],
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
        case "authenticated": {
            // CLI auth confirmed but no bundle row yet. Transition
            // into `authenticated`, capture email for the SaveBundle
            // panel's prefill. Session id stays — it's needed for
            // the subsequent `auth.savebundle` RPC.
            const email = status.email ?? "";
            return {
                state: {
                    ...state,
                    kind: "authenticated",
                    email,
                    authUrl: "",
                    deviceCode: null,
                    error: "",
                },
                events: [{ type: "authenticated", email }],
            };
        }
        case "success": {
            // Post-save terminal: bundle row exists. Reachable from
            // the API-key fast path (where the backend persists in
            // the same RPC) or as a defensive late-Polled landing
            // after BundleSaved. The OAuth path goes through
            // `authenticated` → SaveBundleClicked → BundleSaved → ready.
            //
            // Issue #1624 PR-C Part B: a direct-account session (no
            // bundle involved) reports its result via `status.accountId`
            // instead of `status.bundleId` (which is `""` in that
            // mode). `AuthState.bundleId`/the `succeeded` event's
            // `bundleId` are deliberately NOT renamed here — this
            // reducer has too much regression-pinned history (P1/P2
            // fixes from #845/#847/#849/#850/#853/#981) to risk a
            // sweeping rename for this PR. The field just carries
            // "whatever terminal id this session produced" — an
            // account id in direct-account mode, a bundle id
            // otherwise. The caller (AgentLaunchModal) knows which
            // mode it's in and treats the value accordingly.
            const terminalId = status.accountId || status.bundleId;
            return {
                state: {
                    ...state,
                    kind: "ready",
                    bundleId: terminalId,
                    sessionId: "",
                    authUrl: "",
                    deviceCode: null,
                    error: "",
                },
                events: [
                    {
                        type: "succeeded",
                        bundleId: terminalId,
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
