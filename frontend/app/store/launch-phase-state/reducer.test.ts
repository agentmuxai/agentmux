// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { update } from "./reducer";
import {
    initialState,
    LOGIN_LINK_CAPTURE_LABEL_MS,
    type LaunchPhaseCommand,
    type LaunchPhaseEvent,
    type LaunchPhaseState,
    type LoginFailure,
    type LoginVia,
} from "./types";

const NOW = 1_000_000;

/** Apply commands in order; return the final state and every event. */
function run(cmds: LaunchPhaseCommand[], from: LaunchPhaseState = initialState()) {
    let state = from;
    const events: LaunchPhaseEvent[] = [];
    for (const cmd of cmds) {
        const r = update(state, cmd, NOW);
        state = r.state;
        events.push(...r.events);
    }
    return { state, events };
}

/** A state with login #1 in flight. */
function loginInFlight(via: LoginVia = "relogin", retryAfterLogin = false, from = initialState()) {
    return run([{ type: "LoginRequested", via, retryAfterLogin }], from).state;
}

const types = (events: LaunchPhaseEvent[]) => events.map((e) => e.type);

describe("launch-phase-state: purity", () => {
    it("never mutates its input", () => {
        const s = loginInFlight();
        const frozen = JSON.stringify(s);
        update(s, { type: "LoginSucceeded", id: 1 }, NOW);
        update(s, { type: "LoginEnded", id: 1 }, NOW);
        update(s, { type: "CancelRequested" }, NOW);
        expect(JSON.stringify(s)).toBe(frozen);
    });

    it("returns the same reference for a no-op", () => {
        const s = initialState();
        expect(update(s, { type: "AuthUrlSet", url: null }, NOW).state).toBe(s);
        expect(update(s, { type: "AuthNoticeSet", notice: null }, NOW).state).toBe(s);
        expect(update(s, { type: "CancelReset" }, NOW).state).toBe(s);
        expect(update(s, { type: "MountEnded" }, NOW).state).toBe(s);
    });
});

describe("launch-phase-state: the mount-time launch flow", () => {
    it("MountStarted clears the previous attempt's retry, notice and phase", () => {
        const from = { ...initialState(), canRetry: true, authNotice: "old", cancelled: true, phase: { kind: "verifying" } as const };
        const { state, events } = run([{ type: "MountStarted" }], from);
        expect(state).toMatchObject({ mountRunning: true, canRetry: false, authNotice: null, cancelled: false, phase: null });
        expect(types(events)).toEqual(["mount-started"]);
    });

    it("a second MountStarted while running is ignored (double-click on Retry)", () => {
        const running = run([{ type: "MountStarted" }]).state;
        const r = update(running, { type: "MountStarted" }, NOW);
        expect(r.state).toBe(running);
        expect(r.events).toEqual([{ type: "mount-start-ignored", reason: "already-running" }]);
    });

    it("reports phases only while running", () => {
        const idle = initialState();
        const dropped = update(idle, { type: "MountPhaseReported", phase: { kind: "resolving-cli" } }, NOW);
        expect(dropped.state).toBe(idle);
        expect(dropped.events[0]).toMatchObject({ type: "mount-command-dropped", reason: "not-running" });

        const { state } = run([{ type: "MountStarted" }, { type: "MountPhaseReported", phase: { kind: "checking-auth" } }]);
        expect(state.phase).toEqual({ kind: "checking-auth" });
    });

    it("an equal phase keeps the existing object", () => {
        const s = run([{ type: "MountStarted" }, { type: "MountPhaseReported", phase: { kind: "verifying" } }]).state;
        expect(update(s, { type: "MountPhaseReported", phase: { kind: "verifying" } }, NOW).state).toBe(s);
    });

    it.each([
        ["success", true, { agentReady: true, authStatus: "authenticated", canRetry: false }],
        ["success", false, { agentReady: true, authStatus: "unknown", canRetry: false }],
        ["auth_failed", false, { agentReady: true, authStatus: "unauthenticated", canRetry: true }],
        ["error", false, { agentReady: true, authStatus: "unknown", canRetry: false }],
        ["fatal", false, { agentReady: false, authStatus: "unknown", canRetry: false }],
    ] as const)("MountFinished(%s, confirmed=%s)", (result, authConfirmed, expected) => {
        const { state, events } = run([{ type: "MountStarted" }, { type: "MountFinished", result, authConfirmed }]);
        expect(state).toMatchObject({ ...expected, mountRunning: true });
        expect(events).toContainEqual({ type: "mount-finished", result });
    });

    it("auth_failed after a cancel leaves the not-signed-in state unset", () => {
        const { state } = run([
            { type: "MountStarted" },
            { type: "CancelRequested" },
            { type: "MountFinished", result: "auth_failed", authConfirmed: false },
        ]);
        expect(state.canRetry).toBe(false);
        expect(state.agentReady).toBe(false);
    });

    it("MountEnded stops the flow and clears its phase", () => {
        const { state, events } = run([
            { type: "MountStarted" },
            { type: "MountPhaseReported", phase: { kind: "verifying" } },
            { type: "MountFinished", result: "success", authConfirmed: true },
            { type: "MountEnded" },
        ]);
        expect(state).toMatchObject({ mountRunning: false, phase: null, agentReady: true });
        expect(types(events)).toContain("mount-ended");
    });

    it("MountFinished without a running flow is dropped", () => {
        const s = initialState();
        expect(update(s, { type: "MountFinished", result: "success", authConfirmed: true }, NOW).state).toBe(s);
    });
});

describe("launch-phase-state: starting a pane login", () => {
    it("relogin clears the Log in state, the URL box and the notice, and holds a recovery slot", () => {
        const from = { ...initialState(), canRetry: true, authUrl: "https://old", authNotice: "old", cancelled: true };
        const { state, events } = run([{ type: "LoginRequested", via: "relogin", retryAfterLogin: false }], from);
        expect(state).toMatchObject({
            canRetry: false,
            authUrl: null,
            authNotice: null,
            cancelled: false,
            recoveryFlows: 1,
            retryIntent: false,
            phase: { kind: "checking-auth" },
            login: { id: 1, via: "relogin", retryAfterLogin: false, holdsRecoverySlot: true, succeeded: false },
        });
        expect(events).toEqual([{ type: "login-started", id: 1, via: "relogin" }]);
    });

    it("a terminal login keeps canRetry and the URL box until it succeeds", () => {
        const from = { ...initialState(), canRetry: true, authUrl: "https://old" };
        const state = loginInFlight("terminal", true, from);
        expect(state).toMatchObject({ canRetry: true, authUrl: "https://old", phase: { kind: "opening-login-terminal" }, retryIntent: true });
    });

    it("a second request while one is in flight is ignored (double-click)", () => {
        const s = loginInFlight("relogin", true);
        for (const via of ["relogin", "terminal"] as const) {
            const r = update(s, { type: "LoginRequested", via, retryAfterLogin: false }, NOW);
            expect(r.state).toBe(s);
            expect(r.events).toEqual([{ type: "login-start-ignored", reason: "in-flight" }]);
        }
        // The rejected call's intent never lands.
        expect(s.retryIntent).toBe(true);
    });

    it("ids increase across attempts", () => {
        const { state } = run([
            { type: "LoginRequested", via: "relogin", retryAfterLogin: false },
            { type: "LoginEnded", id: 1 },
            { type: "LoginRequested", via: "terminal", retryAfterLogin: false },
        ]);
        expect(state.login?.id).toBe(2);
    });
});

describe("launch-phase-state: login steps → footer phase", () => {
    it.each([
        ["relogin", { step: "resolving-cli" }, { kind: "resolving-cli" }],
        ["relogin", { step: "cli-ready" }, { kind: "checking-auth" }],
        ["relogin", { step: "provider-login", linkCapture: true }, { kind: "waiting-for-login-link", deadlineMs: NOW + LOGIN_LINK_CAPTURE_LABEL_MS }],
        ["relogin", { step: "provider-login", linkCapture: false }, { kind: "opening-login-terminal" }],
        ["relogin", { step: "fallback" }, { kind: "opening-login-terminal" }],
        ["relogin", { step: "awaiting-completion", deadlineMs: 42 }, { kind: "waiting-for-login-completion", deadlineMs: 42 }],
        ["terminal", { step: "resolving-cli" }, { kind: "resolving-cli" }],
        ["terminal", { step: "cli-ready" }, { kind: "opening-login-terminal" }],
        ["terminal", { step: "provider-login", linkCapture: false }, { kind: "opening-login-terminal" }],
        ["terminal", { step: "awaiting-completion", deadlineMs: 42 }, { kind: "waiting-for-login-completion", deadlineMs: 42 }],
    ] as const)("%s %j", (via, step, phase) => {
        const s = loginInFlight(via);
        const { state } = run([{ type: "LoginStepReported", id: 1, ...step }], s);
        expect(state.phase).toEqual(phase);
    });

    it("a step from a stale attempt is dropped", () => {
        const s = run([
            { type: "LoginRequested", via: "relogin", retryAfterLogin: false },
            { type: "LoginEnded", id: 1 },
            { type: "LoginRequested", via: "terminal", retryAfterLogin: false },
        ]).state;
        const r = update(s, { type: "LoginStepReported", id: 1, step: "fallback" }, NOW);
        expect(r.state).toBe(s);
        expect(r.events).toEqual([{ type: "login-step-dropped", id: 1, reason: "stale" }]);
    });
});

describe("launch-phase-state: cancel mid-login", () => {
    it("clears the phase at once, and later steps don't bring it back", () => {
        const s = run([
            { type: "LoginRequested", via: "relogin", retryAfterLogin: false },
            { type: "LoginStepReported", id: 1, step: "provider-login", linkCapture: true },
            { type: "CancelRequested" },
        ]).state;
        expect(s).toMatchObject({ cancelled: true, phase: null });
        const r = update(s, { type: "LoginStepReported", id: 1, step: "fallback" }, NOW);
        expect(r.state).toBe(s);
        expect(r.events).toEqual([{ type: "login-step-dropped", id: 1, reason: "cancelled" }]);
    });

    it.each([
        ["relogin", { kind: "not-detected", stage: "link" }],
        ["relogin", { kind: "not-detected", stage: "inapp" }],
        ["relogin", { kind: "not-detected", stage: "terminal" }],
        ["terminal", { kind: "not-detected", stage: "terminal" }],
    ] as const)("a %s timeout after a cancel shows no notice (%j)", (via, failure) => {
        const { state, events } = run([
            { type: "LoginRequested", via, retryAfterLogin: true },
            { type: "CancelRequested" },
            { type: "LoginFailed", id: 1, failure },
        ]);
        expect(state.authNotice).toBeNull();
        expect(events).toContainEqual({ type: "login-failed", id: 1, failure, notice: null });
    });

    it("a cancelled in-app session still drops its dead URL box", () => {
        const { state } = run([
            { type: "LoginRequested", via: "relogin", retryAfterLogin: false },
            { type: "AuthUrlSet", url: "https://claude.com/authorize" },
            { type: "CancelRequested" },
            { type: "LoginFailed", id: 1, failure: { kind: "not-detected", stage: "inapp" } },
        ]);
        expect(state.authUrl).toBeNull();
    });

    it("a credential that landed despite the cancel still counts", () => {
        const { state } = run([
            { type: "LoginRequested", via: "relogin", retryAfterLogin: false },
            { type: "CancelRequested" },
            { type: "LoginSucceeded", id: 1 },
        ]);
        expect(state.authStatus).toBe("authenticated");
    });

    it("cancelling a pre-launch relogin puts the Log in state back when it ends", () => {
        const { state } = run([
            { type: "LoginRequested", via: "relogin", retryAfterLogin: false },
            { type: "CancelRequested" },
            { type: "LoginFailed", id: 1, failure: { kind: "not-detected", stage: "link" } },
            { type: "LoginEnded", id: 1 },
        ]);
        expect(state).toMatchObject({ canRetry: true, login: null, recoveryFlows: 0 });
    });

    it("the next login clears the cancel", () => {
        const { state } = run([
            { type: "LoginRequested", via: "relogin", retryAfterLogin: false },
            { type: "CancelRequested" },
            { type: "LoginEnded", id: 1 },
            { type: "LoginRequested", via: "terminal", retryAfterLogin: false },
        ]);
        expect(state.cancelled).toBe(false);
    });

    it("CancelReset clears a cancel left by an earlier attempt (/login starting)", () => {
        const { state } = run([{ type: "CancelRequested" }, { type: "CancelReset" }]);
        expect(state.cancelled).toBe(false);
    });
});

describe("launch-phase-state: success", () => {
    it("clears every not-signed-in signal", () => {
        const from = { ...initialState(), canRetry: true, authNotice: "x", authUrl: "https://u" };
        const s = loginInFlight("terminal", true, from);
        const { state, events } = run([{ type: "LoginSucceeded", id: 1 }], s);
        expect(state).toMatchObject({
            canRetry: false,
            authNotice: null,
            authUrl: null,
            authStatus: "authenticated",
            login: { succeeded: true, holdsRecoverySlot: true },
            recoveryFlows: 1,
        });
        expect(events).toEqual([{ type: "login-succeeded", id: 1 }]);
    });

    it("releasing the slot early drops loginWaiting before the retry, once", () => {
        const s = run([
            { type: "LoginRequested", via: "relogin", retryAfterLogin: true },
            { type: "LoginSucceeded", id: 1 },
            { type: "LoginRecoveryReleased", id: 1 },
        ]).state;
        expect(s).toMatchObject({ recoveryFlows: 0, retryIntent: false, login: { holdsRecoverySlot: false } });
        const again = update(s, { type: "LoginRecoveryReleased", id: 1 }, NOW);
        expect(again.state).toBe(s);
        expect(again.events).toEqual([{ type: "recovery-release-ignored", id: 1, reason: "already-released" }]);
        // LoginEnded doesn't decrement a second time.
        const ended = run([{ type: "LoginEnded", id: 1 }], s).state;
        expect(ended.recoveryFlows).toBe(0);
    });

    it("a succeeded pre-launch relogin does not restore canRetry", () => {
        const { state, events } = run([
            { type: "LoginRequested", via: "relogin", retryAfterLogin: false },
            { type: "LoginSucceeded", id: 1 },
            { type: "LoginEnded", id: 1 },
        ]);
        expect(state.canRetry).toBe(false);
        expect(events).toContainEqual({ type: "login-ended", id: 1, succeeded: true, canRetryRestored: false });
    });
});

describe("launch-phase-state: failures and timeouts", () => {
    const failed = (via: LoginVia, failure: LoginFailure, from = initialState()) =>
        run([{ type: "LoginRequested", via, retryAfterLogin: true }, { type: "LoginFailed", id: 1, failure }], from).state;

    it.each([
        ["relogin", { kind: "not-detected", stage: "link" }, /Opened a login page, but no login was detected within 5 minutes/],
        ["relogin", { kind: "not-detected", stage: "inapp" }, /login link timed out/],
        ["relogin", { kind: "not-detected", stage: "terminal" }, /Opened a terminal window for login, but no login was detected/],
        ["relogin", { kind: "unavailable" }, /Couldn't start a browser login or open a terminal window/],
        ["relogin", { kind: "save-failed" }, /couldn't save the account record/],
        ["relogin", { kind: "error", message: "Re-login failed: boom" }, /^Re-login failed: boom$/],
        ["terminal", { kind: "not-detected", stage: "terminal" }, /^No login detected after 5 minutes/],
        ["terminal", { kind: "unavailable" }, /^Couldn't open a terminal window for login on this platform\.$/],
        ["terminal", { kind: "save-failed" }, /couldn't save the account record/],
        ["terminal", { kind: "error", message: "Terminal login failed: boom" }, /^Terminal login failed: boom$/],
    ] as const)("%s %j → notice", (via, failure, notice) => {
        expect(failed(via, failure).authNotice).toMatch(notice);
    });

    it("a terminal login has no link stages: no notice for them", () => {
        expect(failed("terminal", { kind: "not-detected", stage: "link" }).authNotice).toBeNull();
    });

    it("save-failed marks the agent signed out; a relogin also drops its URL box", () => {
        const from = { ...initialState(), authStatus: "authenticated" as const };
        const relogin = run([
            { type: "LoginRequested", via: "relogin", retryAfterLogin: true },
            { type: "AuthUrlSet", url: "https://u" },
            { type: "LoginFailed", id: 1, failure: { kind: "save-failed" } },
        ], from).state;
        expect(relogin).toMatchObject({ authStatus: "unauthenticated", authUrl: null });

        const terminal = run([
            { type: "LoginRequested", via: "terminal", retryAfterLogin: true },
            { type: "AuthUrlSet", url: "https://u" },
            { type: "LoginFailed", id: 1, failure: { kind: "save-failed" } },
        ], from).state;
        expect(terminal).toMatchObject({ authStatus: "unauthenticated", authUrl: "https://u" });
    });

    it("a link timeout leaves the URL box (the page may still be open)", () => {
        const { state } = run([
            { type: "LoginRequested", via: "relogin", retryAfterLogin: true },
            { type: "AuthUrlSet", url: "https://u" },
            { type: "LoginFailed", id: 1, failure: { kind: "not-detected", stage: "link" } },
        ]);
        expect(state.authUrl).toBe("https://u");
    });

    it.each([
        ["relogin", false, true],
        ["relogin", true, false],
        ["terminal", false, false],
        ["terminal", true, false],
    ] as const)("a failed %s (retryAfterLogin=%s) ending restores canRetry: %s", (via, retryAfterLogin, restored) => {
        const { state, events } = run([
            { type: "LoginRequested", via, retryAfterLogin },
            { type: "LoginFailed", id: 1, failure: { kind: "unavailable" } },
            { type: "LoginEnded", id: 1 },
        ]);
        expect(state.canRetry).toBe(restored);
        expect(state).toMatchObject({ login: null, phase: null, recoveryFlows: 0 });
        expect(events).toContainEqual({ type: "login-ended", id: 1, succeeded: false, canRetryRestored: restored });
    });

    it("a terminal login that fails keeps an existing canRetry", () => {
        const from = { ...initialState(), canRetry: true };
        const { state } = run([
            { type: "LoginRequested", via: "terminal", retryAfterLogin: false },
            { type: "LoginFailed", id: 1, failure: { kind: "unavailable" } },
            { type: "LoginEnded", id: 1 },
        ], from);
        expect(state.canRetry).toBe(true);
    });

    it("outcomes for a stale id are dropped", () => {
        const s = run([
            { type: "LoginRequested", via: "relogin", retryAfterLogin: false },
            { type: "LoginEnded", id: 1 },
        ]).state;
        for (const cmd of [
            { type: "LoginSucceeded", id: 1 },
            { type: "LoginFailed", id: 1, failure: { kind: "unavailable" } },
            { type: "LoginEnded", id: 1 },
        ] as const) {
            const r = update(s, cmd, NOW);
            expect(r.state).toBe(s);
            expect(r.events[0]).toMatchObject({ type: "login-command-dropped", id: 1, reason: "stale" });
        }
        const rel = update(s, { type: "LoginRecoveryReleased", id: 1 }, NOW);
        expect(rel.events).toEqual([{ type: "recovery-release-ignored", id: 1, reason: "stale" }]);
    });
});

describe("launch-phase-state: the shared recovery counter and intent", () => {
    it("/login (RecoveryBegan) and a pane login share one counter", () => {
        let s = run([{ type: "RecoveryBegan", retryAfterLogin: false }]).state;
        expect(s.recoveryFlows).toBe(1);
        s = loginInFlight("terminal", true, s);
        expect(s.recoveryFlows).toBe(2);
        s = run([{ type: "RecoveryEnded" }], s).state;
        expect(s.recoveryFlows).toBe(1);
        s = run([{ type: "LoginEnded", id: 1 }], s).state;
        expect(s.recoveryFlows).toBe(0);
    });

    it("the first flow's intent wins: /login first, then a row button", () => {
        let s = run([{ type: "RecoveryBegan", retryAfterLogin: false }]).state;
        s = loginInFlight("terminal", true, s);
        expect(s.retryIntent).toBe(false);
    });

    it("the first flow's intent wins: a relogin first, then /login joins", () => {
        let s = loginInFlight("relogin", true);
        s = run([{ type: "RecoveryBegan", retryAfterLogin: false }], s).state;
        expect(s.retryIntent).toBe(true);
    });

    it("a flow that declares nothing (a bind) leaves the intent alone", () => {
        const s = run([{ type: "RecoveryBegan" }]).state;
        expect(s).toMatchObject({ recoveryFlows: 1, retryIntent: false });
    });

    it("the intent resets once nothing is in flight", () => {
        const { state } = run([
            { type: "LoginRequested", via: "relogin", retryAfterLogin: true },
            { type: "LoginEnded", id: 1 },
        ]);
        expect(state.retryIntent).toBe(false);
    });

    it("the counter never goes below zero", () => {
        const { state } = run([{ type: "RecoveryEnded" }, { type: "RecoveryEnded" }]);
        expect(state.recoveryFlows).toBe(0);
    });

    it("a relogin while a bind or /login is already in flight starts, and does not end theirs", () => {
        const { state } = run([
            { type: "RecoveryBegan" },
            { type: "LoginRequested", via: "relogin", retryAfterLogin: false },
            { type: "LoginSucceeded", id: 1 },
            { type: "LoginRecoveryReleased", id: 1 },
            { type: "LoginEnded", id: 1 },
        ]);
        // The other flow's slot is still held.
        expect(state.recoveryFlows).toBe(1);
    });

    it("a relogin during the mount flow: the mount's end clears the phase, and its outcome still applies", () => {
        const { state } = run([
            { type: "MountStarted" },
            { type: "LoginRequested", via: "relogin", retryAfterLogin: true },
            { type: "MountFinished", result: "success", authConfirmed: false },
            { type: "MountEnded" },
        ]);
        expect(state).toMatchObject({ mountRunning: false, phase: null, agentReady: true, login: { id: 1 } });
    });
});

describe("launch-phase-state: other commands", () => {
    it("ControllerHealthy clears the not-signed-in state", () => {
        const from = { ...initialState(), canRetry: true, authNotice: "x", authStatus: "unauthenticated" as const };
        const { state } = run([{ type: "ControllerHealthy" }], from);
        expect(state).toMatchObject({ canRetry: false, authNotice: null, authStatus: "authenticated" });
        expect(update(state, { type: "ControllerHealthy" }, NOW).state).toBe(state);
    });

    it("AuthUrlSet / AuthNoticeSet set their field", () => {
        const { state } = run([
            { type: "AuthUrlSet", url: "https://u" },
            { type: "AuthNoticeSet", notice: "n" },
        ]);
        expect(state).toMatchObject({ authUrl: "https://u", authNotice: "n" });
    });

    it("after Disposed every command is a no-op, and the flow reads as cancelled", () => {
        const s = run([{ type: "LoginRequested", via: "relogin", retryAfterLogin: false }, { type: "Disposed" }]).state;
        expect(s.cancelled).toBe(true);
        const r = update(s, { type: "LoginEnded", id: 1 }, NOW);
        expect(r.state).toBe(s);
        expect(r.events).toEqual([{ type: "command-ignored", command: "LoginEnded", reason: "disposed" }]);
        expect(update(s, { type: "CancelReset" }, NOW).state.cancelled).toBe(true);
    });
});
