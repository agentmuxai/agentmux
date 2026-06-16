// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

import { failureToRow, isTransient, type FailureActions, type FailureViewState } from "./failure-accessory";

const mkFailure = (overrides: Partial<AgentFailure> = {}): AgentFailure => ({
    code: "unknown_non_zero",
    title: "Agent run failed",
    detail: "The agent exited with a non-zero status.",
    retryable: true,
    ...overrides,
});

const mkView = (overrides: Partial<FailureViewState> = {}): FailureViewState => ({
    expanded: false,
    autoRetryIn: null,
    retrying: false,
    ...overrides,
});

const mkActions = (): FailureActions & { _calls: Record<keyof FailureActions, number> } => {
    const _calls = { retry: 0, loginAgain: 0, trustCenter: 0, toggleDetails: 0, dismiss: 0 };
    return {
        _calls,
        retry: vi.fn(() => void _calls.retry++),
        loginAgain: vi.fn(() => void _calls.loginAgain++),
        trustCenter: vi.fn(() => void _calls.trustCenter++),
        toggleDetails: vi.fn(() => void _calls.toggleDetails++),
        dismiss: vi.fn(() => void _calls.dismiss++),
    };
};

/** Find the first action whose label (or glyph, for the bare Dismiss) matches. */
const action = (row: ReturnType<typeof failureToRow>, label: string) =>
    row.actions.find((a) => a.label === label || a.glyph === label);

describe("isTransient", () => {
    it("is true only for throttling classes safe to auto-retry", () => {
        expect(isTransient("rate_limited")).toBe(true);
        expect(isTransient("overloaded")).toBe(true);
        expect(isTransient("network")).toBe(true);
    });

    it("is false for classes that need user action", () => {
        for (const code of ["auth", "usage_limit", "context_exceeded", "max_turns", "killed", "spawn_failure", "no_output", "unknown_non_zero"] as const) {
            expect(isTransient(code)).toBe(false);
        }
    });
});

describe("failureToRow", () => {
    it("always renders an error accent and a Dismiss action wired to dismiss", () => {
        const on = mkActions();
        const row = failureToRow(mkFailure(), mkView(), on);
        expect(row.accent).toBe("error");
        const dismiss = action(row, "×");
        expect(dismiss?.danger).toBe(true);
        dismiss?.onClick();
        expect(on._calls.dismiss).toBe(1);
    });

    it("auth → 🔐 sigil with Login Again (re-auth) + Trust Center actions", () => {
        const on = mkActions();
        const row = failureToRow(mkFailure({ code: "auth", title: "Not authenticated" }), mkView(), on);
        expect(row.sigil).toBe("🔐");
        expect(row.title).toBe("Not authenticated");

        const login = action(row, "Login Again");
        expect(login?.primary).toBe(true);
        login?.onClick();
        expect(on._calls.loginAgain).toBe(1);

        const trust = action(row, "Trust Center → Accounts");
        expect(trust).toBeTruthy();
        trust?.onClick();
        expect(on._calls.trustCenter).toBe(1);
    });

    it("usage_limit → Trust Center is the primary action", () => {
        const row = failureToRow(mkFailure({ code: "usage_limit" }), mkView(), mkActions());
        const trust = action(row, "Trust Center (switch / upgrade)");
        expect(trust?.primary).toBe(true);
    });

    it("transient classes get a retry action wired to retry()", () => {
        const on = mkActions();
        const row = failureToRow(mkFailure({ code: "rate_limited" }), mkView(), on);
        const retry = action(row, "Retry now");
        expect(retry?.primary).toBe(true);
        retry?.onClick();
        expect(on._calls.retry).toBe(1);
    });

    it("retry label shows the live countdown when auto-retry is armed", () => {
        const row = failureToRow(mkFailure({ code: "overloaded" }), mkView({ autoRetryIn: 5 }), mkActions());
        expect(action(row, "Retry now (5s)")).toBeTruthy();
    });

    it("retry is disabled while a retry is in flight", () => {
        const row = failureToRow(mkFailure({ code: "network" }), mkView({ retrying: true }), mkActions());
        expect(action(row, "Retry now")?.disabled).toBe(true);
    });

    it("only surfaces a Details toggle when there is a stderr tail", () => {
        const on = mkActions();
        const without = failureToRow(mkFailure(), mkView(), on);
        expect(action(without, "Details")).toBeUndefined();

        const withTail = failureToRow(mkFailure({ stderrTail: "panic: boom" }), mkView(), on);
        const details = action(withTail, "Details");
        expect(details).toBeTruthy();
        details?.onClick();
        expect(on._calls.toggleDetails).toBe(1);
        expect(withTail.stderrTail).toBe("panic: boom");
    });

    it("Details toggle flips its label/glyph when expanded", () => {
        const row = failureToRow(mkFailure({ stderrTail: "x" }), mkView({ expanded: true }), mkActions());
        const hide = action(row, "Hide details");
        expect(hide?.glyph).toBe("▾");
    });

    it("meta prefers signal over exit code and flags retryable", () => {
        const sigRow = failureToRow(mkFailure({ code: "killed", signal: 9, exitCode: 137, retryable: false }), mkView(), mkActions());
        expect(sigRow.meta).toBe("killed · signal 9");

        const exitRow = failureToRow(mkFailure({ code: "unknown_non_zero", exitCode: 1, retryable: true }), mkView(), mkActions());
        expect(exitRow.meta).toBe("unknown_non_zero · exit 1 · retryable");
    });

    it("falls back to a generic Retry for unclassified non-zero exits", () => {
        const on = mkActions();
        const row = failureToRow(mkFailure({ code: "unknown_non_zero" }), mkView(), on);
        const retry = action(row, "Retry");
        expect(retry).toBeTruthy();
        retry?.onClick();
        expect(on._calls.retry).toBe(1);
    });
});
