// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Types for the launch-phase-state slice: an agent pane's post-mount launch
 * and login state (the mount-time launch flow, the pane's own login
 * attempts, and the recovery counter `/login` and binds share).
 *
 * A sibling of `launch-flow-state`, not part of it: that slice is the
 * pre-launch modal, which runs before any pane exists.
 * REPORT_REDUCER_STACK_AUDIT_2026_07_26.md §4.4 explains why the two stay
 * separate. Conventions: docs/specs/frontend-reducer-conventions-2026-05-03.md.
 */

import type { LaunchPhase } from "@/app/view/agent/flows/launch-phase";

/** Display-only estimate of tier 1's link-capture wait, shown as the
 *  `waiting-for-login-link` countdown. Must track cli_login.rs's
 *  URL_CAPTURE_TIMEOUT_SECS (15 s) by hand: the frontend can't read it. */
export const LOGIN_LINK_CAPTURE_LABEL_MS = 15_000;

/** Durable logged-in/logged-out state for the composer strip's tag.
 *  "unknown" until the first auth check resolves, or after a launch whose
 *  check never ran or couldn't confirm. */
export type AuthStatus = "authenticated" | "unauthenticated" | "unknown";

/** Which pane action started a login: "Log in" / "Login Again" (relogin),
 *  or "Login via terminal". */
export type LoginVia = "relogin" | "terminal";

/** One login the pane itself is running. At most one at a time. */
export interface LoginAttempt {
    id: number;
    via: LoginVia;
    /** Retry the failed turn on success (true), or send the startup
     *  sequence instead (false: no turn ever ran). */
    retryAfterLogin: boolean;
    /** Still counted in `recoveryFlows`. Released early on success so the
     *  retried send doesn't see `loginWaiting()` still true. */
    holdsRecoverySlot: boolean;
    succeeded: boolean;
}

export interface LaunchPhaseState {
    /** The pane unmounted. Every later command is a no-op. */
    disposed: boolean;
    /** What the flow is doing right now, for the footer label. */
    phase: LaunchPhase | null;
    /** The mount-time launch flow is running. */
    mountRunning: boolean;
    /** The mount flow got far enough that its spinner should clear. */
    agentReady: boolean;
    /** The agent is known to be signed out and no login is under way:
     *  sends fast-fail, and the failure row offers "Log in". */
    canRetry: boolean;
    authStatus: AuthStatus;
    /** Login link shown in the AuthUrlBox. */
    authUrl: string | null;
    /** Auth-recovery notice shown above the composer. */
    authNotice: string | null;
    login: LoginAttempt | null;
    nextLoginId: number;
    /** Recovery flows in flight: the pane's own login plus any `/login` or
     *  bind. `loginWaiting()` is `recoveryFlows > 0`. */
    recoveryFlows: number;
    /** `retryAfterLogin` of the first recovery flow still in flight, read by
     *  "Use terminal instead". The first flow wins, and it resets to false
     *  once nothing is in flight. */
    retryIntent: boolean;
    /** The user cancelled the login in flight (or the pane closed). Reset
     *  when the next login starts. */
    cancelled: boolean;
}

export const initialState = (): LaunchPhaseState => ({
    disposed: false,
    phase: null,
    mountRunning: false,
    agentReady: false,
    canRetry: false,
    authStatus: "unknown",
    authUrl: null,
    authNotice: null,
    login: null,
    nextLoginId: 1,
    recoveryFlows: 0,
    retryIntent: false,
    cancelled: false,
});

/** How `runLaunchFlow` ended; "error" is a throw out of it. */
export type MountResult = "success" | "auth_failed" | "fatal" | "error";

/** Progress inside a pane login, mapped to a footer phase by the reducer. */
export type LoginStep =
    | { step: "resolving-cli" }
    | { step: "cli-ready" }
    /** The provider login is about to run. `linkCapture`: tier 1 will try
     *  to capture a login link first (relogin only). */
    | { step: "provider-login"; linkCapture: boolean }
    /** Tier 1 gave up; a terminal is opening. */
    | { step: "fallback" }
    /** A login page or terminal is open; waiting for it to finish. */
    | { step: "awaiting-completion"; deadlineMs: number };

/** Why a pane login didn't sign in. Mirrors `LoginConfirmation` from
 *  flows/login-and-confirm.ts, plus "error" for a thrown failure or a CLI
 *  that couldn't be found (the hook supplies the text). */
export type LoginFailure =
    | { kind: "save-failed" }
    | { kind: "not-detected"; stage: "link" | "inapp" | "terminal" }
    | { kind: "unavailable" }
    | { kind: "error"; message: string };

export type LaunchPhaseCommand =
    | { type: "MountStarted" }
    | { type: "MountPhaseReported"; phase: LaunchPhase | null }
    /** The flow returned. Sets the outcome; `MountEnded` follows once the
     *  caller has run `onReady`, which must still see the flow running. */
    | { type: "MountFinished"; result: MountResult; authConfirmed: boolean }
    | { type: "MountEnded" }
    | { type: "LoginRequested"; via: LoginVia; retryAfterLogin: boolean }
    | ({ type: "LoginStepReported"; id: number } & LoginStep)
    | { type: "LoginSucceeded"; id: number }
    | { type: "LoginFailed"; id: number; failure: LoginFailure }
    /** Give back this login's recovery slot before the retry/startup send. */
    | { type: "LoginRecoveryReleased"; id: number }
    | { type: "LoginEnded"; id: number }
    /** `/login` or a bind joins the shared recovery counter. */
    | { type: "RecoveryBegan"; retryAfterLogin?: boolean }
    | { type: "RecoveryEnded" }
    | { type: "CancelRequested" }
    /** `/login` starting: clear a cancel left by an earlier attempt. */
    | { type: "CancelReset" }
    | { type: "AuthUrlSet"; url: string | null }
    | { type: "AuthNoticeSet"; notice: string | null }
    /** Independent proof the controller is running on a good credential. */
    | { type: "ControllerHealthy" }
    | { type: "Disposed" };

export type LaunchPhaseEvent =
    | { type: "mount-started" }
    | { type: "mount-start-ignored"; reason: "already-running" }
    | { type: "mount-command-dropped"; command: string; reason: "not-running" }
    | { type: "mount-finished"; result: MountResult }
    | { type: "mount-ended" }
    | { type: "login-started"; id: number; via: LoginVia }
    | { type: "login-start-ignored"; reason: "in-flight" }
    | { type: "login-step-dropped"; id: number; reason: "stale" | "cancelled" }
    | { type: "login-succeeded"; id: number }
    | { type: "login-failed"; id: number; failure: LoginFailure; notice: string | null }
    | { type: "recovery-released"; id: number; remaining: number }
    | { type: "recovery-release-ignored"; id: number; reason: "stale" | "already-released" }
    | { type: "login-ended"; id: number; succeeded: boolean; canRetryRestored: boolean }
    | { type: "login-command-dropped"; id: number; command: string; reason: "stale" }
    | { type: "recovery-began"; count: number }
    | { type: "recovery-ended"; count: number }
    | { type: "login-cancelled" }
    | { type: "command-ignored"; command: string; reason: "disposed" };

export interface ReducerResult {
    state: LaunchPhaseState;
    events: LaunchPhaseEvent[];
}
