// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for the launch-phase-state slice: `update(state, command,
 * nowMs) → { state, events }`. Replaces the separate signals and the
 * in-flight, cancelled and intent flags `useAgentControllerStatus` used to
 * keep by hand. A login carries an id, so a step or outcome from an
 * attempt that is no longer current is dropped instead of applied.
 */

import type { LaunchPhase } from "@/app/agent-model/launch-phase";
import { pickHandler } from "@/util/handler-map";

import {
    LOGIN_LINK_CAPTURE_LABEL_MS,
    type LaunchPhaseCommand,
    type LaunchPhaseEvent,
    type LaunchPhaseState,
    type LoginAttempt,
    type LoginFailure,
    type LoginVia,
    type ReducerResult,
} from "./types";

const SAVE_FAILED_NOTICE =
    "Your login succeeded, but AgentMux couldn't save the account record. Try again in a moment.";

/** The notice each failed pane login leaves, and whether a cancel hides it
 *  (the user left the flow; there is nothing wrong to report). */
interface FailureView {
    notice: string | null;
    hiddenByCancel: boolean;
    clearUrl: boolean;
    markSignedOut: boolean;
}

function failureView(via: LoginVia, failure: LoginFailure): FailureView {
    const view = (notice: string | null, hiddenByCancel: boolean, clearUrl = false, markSignedOut = false) => ({
        notice,
        hiddenByCancel,
        clearUrl,
        markSignedOut,
    });
    switch (failure.kind) {
        case "error":
            return view(failure.message, false);
        case "save-failed":
            return view(SAVE_FAILED_NOTICE, false, via === "relogin", true);
        case "unavailable":
            return via === "relogin"
                ? view(
                      "Couldn't start a browser login or open a terminal window on this platform. " +
                          "Sign in to this provider from Connectors → Accounts, then try again.",
                      false,
                  )
                : view("Couldn't open a terminal window for login on this platform.", false);
        case "not-detected":
            if (via === "terminal") {
                // A terminal login has no link stages.
                return failure.stage === "terminal"
                    ? view(
                          "No login detected after 5 minutes. Complete the login in the terminal, then click “Login Again”.",
                          true,
                      )
                    : view(null, true);
            }
            switch (failure.stage) {
                case "link":
                    return view(
                        "Opened a login page, but no login was detected within 5 minutes. " +
                            "Complete the login there, then click “Login Again”.",
                        true,
                    );
                case "inapp":
                    // The login child is already reaped: drop its URL box too.
                    return view(
                        "The login link timed out before you finished authorizing. " +
                            "Click “Login Again” to get a new link, or try “Login via terminal”.",
                        true,
                        true,
                    );
                case "terminal":
                    return view(
                        "Opened a terminal window for login, but no login was detected within 5 minutes. " +
                            "Complete the login in that terminal, then click “Login Again”.",
                        true,
                    );
            }
    }
}

function loginPhaseFor(
    login: LoginAttempt,
    step: Extract<LaunchPhaseCommand, { type: "LoginStepReported" }>,
    current: LaunchPhase | null,
    nowMs: number,
): LaunchPhase | null {
    switch (step.step) {
        case "resolving-cli":
            return { kind: "resolving-cli" };
        case "cli-ready":
            return login.via === "relogin" ? { kind: "checking-auth" } : { kind: "opening-login-terminal" };
        case "provider-login":
            if (login.via === "terminal") return current;
            return step.linkCapture
                ? { kind: "waiting-for-login-link", deadlineMs: nowMs + LOGIN_LINK_CAPTURE_LABEL_MS }
                : { kind: "opening-login-terminal" };
        case "fallback":
            return { kind: "opening-login-terminal" };
        case "awaiting-completion":
            return { kind: "waiting-for-login-completion", deadlineMs: step.deadlineMs };
    }
}

/** Give one slot back to the shared recovery counter. The intent belongs to
 *  the flows in flight, so it resets once none are. */
function releaseRecovery(state: LaunchPhaseState): LaunchPhaseState {
    const recoveryFlows = Math.max(0, state.recoveryFlows - 1);
    return { ...state, recoveryFlows, retryIntent: recoveryFlows === 0 ? false : state.retryIntent };
}

/** Join the shared recovery counter. The first flow in records the intent;
 *  a flow joining a running one cannot redefine what it is recovering. */
function beginRecovery(state: LaunchPhaseState, retryAfterLogin: boolean | undefined): LaunchPhaseState {
    const retryIntent =
        retryAfterLogin !== undefined && state.recoveryFlows === 0 ? retryAfterLogin : state.retryIntent;
    return { ...state, recoveryFlows: state.recoveryFlows + 1, retryIntent };
}

const same = (state: LaunchPhaseState, ...events: LaunchPhaseEvent[]): ReducerResult => ({ state, events });

/** Equal phases keep the old object, so the footer doesn't re-render. */
function samePhase(a: LaunchPhase | null, b: LaunchPhase | null): boolean {
    if (a === b) return true;
    if (!a || !b || a.kind !== b.kind) return false;
    return ("deadlineMs" in a ? a.deadlineMs : undefined) === ("deadlineMs" in b ? b.deadlineMs : undefined) &&
        ("reason" in a ? a.reason : undefined) === ("reason" in b ? b.reason : undefined);
}

type Cmd<T extends LaunchPhaseCommand["type"]> = Extract<LaunchPhaseCommand, { type: T }>;

type Handlers = {
    [T in LaunchPhaseCommand["type"]]: (state: LaunchPhaseState, command: Cmd<T>, nowMs: number) => ReducerResult;
};

// Commands: one function each, picked by `update` through `HANDLERS`.

function onMountStarted(state: LaunchPhaseState): ReducerResult {
    if (state.mountRunning) return same(state, { type: "mount-start-ignored", reason: "already-running" });
    return {
        state: { ...state, mountRunning: true, cancelled: false, canRetry: false, authNotice: null, phase: null },
        events: [{ type: "mount-started" }],
    };
}

function onMountPhaseReported(state: LaunchPhaseState, command: Cmd<"MountPhaseReported">): ReducerResult {
    if (!state.mountRunning) return same(state, { type: "mount-command-dropped", command: command.type, reason: "not-running" });
    if (samePhase(state.phase, command.phase)) return same(state);
    return { state: { ...state, phase: command.phase }, events: [] };
}

function onMountFinished(state: LaunchPhaseState, command: Cmd<"MountFinished">): ReducerResult {
    if (!state.mountRunning) return same(state, { type: "mount-command-dropped", command: command.type, reason: "not-running" });
    let next = state;
    switch (command.result) {
        case "success":
            // "success" means the controller registered, not that the
            // auth check proved the login.
            next = { ...state, agentReady: true, authStatus: command.authConfirmed ? "authenticated" : "unknown" };
            break;
        case "auth_failed":
            if (!state.cancelled) {
                next = { ...state, canRetry: true, agentReady: true, authStatus: "unauthenticated" };
            }
            break;
        case "error":
            next = { ...state, agentReady: true };
            break;
        case "fatal":
            break;
    }
    return { state: next, events: [{ type: "mount-finished", result: command.result }] };
}

function onMountEnded(state: LaunchPhaseState): ReducerResult {
    if (!state.mountRunning) return same(state);
    return { state: { ...state, mountRunning: false, phase: null }, events: [{ type: "mount-ended" }] };
}

function onLoginRequested(state: LaunchPhaseState, command: Cmd<"LoginRequested">): ReducerResult {
    if (state.login) return same(state, { type: "login-start-ignored", reason: "in-flight" });
    const login: LoginAttempt = {
        id: state.nextLoginId,
        via: command.via,
        retryAfterLogin: command.retryAfterLogin,
        holdsRecoverySlot: true,
        succeeded: false,
    };
    let next: LaunchPhaseState = {
        ...beginRecovery(state, command.retryAfterLogin),
        login,
        nextLoginId: state.nextLoginId + 1,
        cancelled: false,
        authNotice: null,
    };
    if (command.via === "relogin") {
        // The click hands off from the "Log in" state: a stale
        // canRetry or URL box must not outlive it.
        next = { ...next, canRetry: false, authUrl: null, phase: { kind: "checking-auth" } };
    } else {
        next = { ...next, phase: { kind: "opening-login-terminal" } };
    }
    return { state: next, events: [{ type: "login-started", id: login.id, via: login.via }] };
}

function onLoginStepReported(state: LaunchPhaseState, command: Cmd<"LoginStepReported">, nowMs: number): ReducerResult {
    const login = state.login;
    if (!login || login.id !== command.id) {
        return same(state, { type: "login-step-dropped", id: command.id, reason: "stale" });
    }
    // After a cancel the flow is unwinding; its label must not come back.
    if (state.cancelled) return same(state, { type: "login-step-dropped", id: command.id, reason: "cancelled" });
    const phase = loginPhaseFor(login, command, state.phase, nowMs);
    if (samePhase(phase, state.phase)) return same(state);
    return { state: { ...state, phase }, events: [] };
}

function onLoginSucceeded(state: LaunchPhaseState, command: Cmd<"LoginSucceeded">): ReducerResult {
    const login = state.login;
    if (!login || login.id !== command.id) {
        return same(state, { type: "login-command-dropped", id: command.id, command: command.type, reason: "stale" });
    }
    return {
        state: {
            ...state,
            login: { ...login, succeeded: true },
            canRetry: false,
            authNotice: null,
            authUrl: null,
            authStatus: "authenticated",
        },
        events: [{ type: "login-succeeded", id: login.id }],
    };
}

function onLoginFailed(state: LaunchPhaseState, command: Cmd<"LoginFailed">): ReducerResult {
    const login = state.login;
    if (!login || login.id !== command.id) {
        return same(state, { type: "login-command-dropped", id: command.id, command: command.type, reason: "stale" });
    }
    const view = failureView(login.via, command.failure);
    const notice = view.hiddenByCancel && state.cancelled ? null : view.notice;
    let next = state;
    if (view.clearUrl) next = { ...next, authUrl: null };
    if (view.markSignedOut) next = { ...next, authStatus: "unauthenticated" };
    if (notice !== null) next = { ...next, authNotice: notice };
    return {
        state: next,
        events: [{ type: "login-failed", id: login.id, failure: command.failure, notice }],
    };
}

function onLoginRecoveryReleased(state: LaunchPhaseState, command: Cmd<"LoginRecoveryReleased">): ReducerResult {
    const login = state.login;
    if (!login || login.id !== command.id) {
        return same(state, { type: "recovery-release-ignored", id: command.id, reason: "stale" });
    }
    if (!login.holdsRecoverySlot) {
        return same(state, { type: "recovery-release-ignored", id: command.id, reason: "already-released" });
    }
    const next = { ...releaseRecovery(state), login: { ...login, holdsRecoverySlot: false } };
    return { state: next, events: [{ type: "recovery-released", id: login.id, remaining: next.recoveryFlows }] };
}

function onLoginEnded(state: LaunchPhaseState, command: Cmd<"LoginEnded">): ReducerResult {
    const login = state.login;
    if (!login || login.id !== command.id) {
        return same(state, { type: "login-command-dropped", id: command.id, command: command.type, reason: "stale" });
    }
    let next = login.holdsRecoverySlot ? releaseRecovery(state) : state;
    // Only the pre-launch "Log in" (relogin, no turn to retry) cleared
    // canRetry on click, so only it puts the not-signed-in state back
    // when it fails. A send guard and the failure row depend on it.
    const canRetryRestored = login.via === "relogin" && !login.succeeded && !login.retryAfterLogin;
    next = { ...next, login: null, phase: null, canRetry: canRetryRestored ? true : next.canRetry };
    return {
        state: next,
        events: [{ type: "login-ended", id: login.id, succeeded: login.succeeded, canRetryRestored }],
    };
}

function onRecoveryBegan(state: LaunchPhaseState, command: Cmd<"RecoveryBegan">): ReducerResult {
    const next = beginRecovery(state, command.retryAfterLogin);
    return { state: next, events: [{ type: "recovery-began", count: next.recoveryFlows }] };
}

function onRecoveryEnded(state: LaunchPhaseState): ReducerResult {
    const next = releaseRecovery(state);
    return { state: next, events: [{ type: "recovery-ended", count: next.recoveryFlows }] };
}

function onCancelRequested(state: LaunchPhaseState): ReducerResult {
    // The phase clears at once; the flow notices the flag on its own
    // next poll tick.
    return { state: { ...state, cancelled: true, phase: null }, events: [{ type: "login-cancelled" }] };
}

function onCancelReset(state: LaunchPhaseState): ReducerResult {
    if (!state.cancelled) return same(state);
    return { state: { ...state, cancelled: false }, events: [] };
}

function onAuthUrlSet(state: LaunchPhaseState, command: Cmd<"AuthUrlSet">): ReducerResult {
    if (state.authUrl === command.url) return same(state);
    return { state: { ...state, authUrl: command.url }, events: [] };
}

function onAuthNoticeSet(state: LaunchPhaseState, command: Cmd<"AuthNoticeSet">): ReducerResult {
    if (state.authNotice === command.notice) return same(state);
    return { state: { ...state, authNotice: command.notice }, events: [] };
}

function onControllerHealthy(state: LaunchPhaseState): ReducerResult {
    if (!state.canRetry && state.authNotice === null && state.authStatus === "authenticated") return same(state);
    return { state: { ...state, canRetry: false, authNotice: null, authStatus: "authenticated" }, events: [] };
}

function onDisposed(state: LaunchPhaseState): ReducerResult {
    return { state: { ...state, disposed: true, cancelled: true }, events: [] };
}

const HANDLERS: Handlers = {
    MountStarted: onMountStarted,
    MountPhaseReported: onMountPhaseReported,
    MountFinished: onMountFinished,
    MountEnded: onMountEnded,
    LoginRequested: onLoginRequested,
    LoginStepReported: onLoginStepReported,
    LoginSucceeded: onLoginSucceeded,
    LoginFailed: onLoginFailed,
    LoginRecoveryReleased: onLoginRecoveryReleased,
    LoginEnded: onLoginEnded,
    RecoveryBegan: onRecoveryBegan,
    RecoveryEnded: onRecoveryEnded,
    CancelRequested: onCancelRequested,
    CancelReset: onCancelReset,
    AuthUrlSet: onAuthUrlSet,
    AuthNoticeSet: onAuthNoticeSet,
    ControllerHealthy: onControllerHealthy,
    Disposed: onDisposed,
};

export function update(state: LaunchPhaseState, command: LaunchPhaseCommand, nowMs: number = Date.now()): ReducerResult {
    if (state.disposed) {
        return same(state, { type: "command-ignored", command: command.type, reason: "disposed" });
    }

    const handle = pickHandler("launch-phase", HANDLERS, command.type) as
        | ((state: LaunchPhaseState, command: LaunchPhaseCommand, nowMs: number) => ReducerResult)
        | undefined;
    if (!handle) return same(state);
    return handle(state, command, nowMs);
}
