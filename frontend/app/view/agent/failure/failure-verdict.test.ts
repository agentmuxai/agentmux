// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { AgentFailure } from "@/types/rpc/AgentFailure";
import { accountStateOf, agoLabel, failureVerdict, isAccountCause, type FailureLive } from "./failure-verdict";

const NOW = 10_000_000;
const AT = NOW - 60_000;
const fail = (code: AgentFailure["code"], extra: Partial<AgentFailure> = {}) => ({
    data: { code, title: code, detail: "", retryable: false, ...extra } as AgentFailure,
    at: AT,
});
const live = (over: Partial<FailureLive> = {}): FailureLive => ({
    account: "unknown",
    completedTurnAt: null,
    ladderEnded: false,
    ...over,
});

describe("accountStateOf", () => {
    it("says can_work only for a live agent, and nothing for a stopped one", () => {
        expect(accountStateOf("idle")).toBe("can_work");
        expect(accountStateOf("working")).toBe("can_work");
        expect(accountStateOf("waiting_on_user")).toBe("can_work");
        expect(accountStateOf("limited")).toBe("limited");
        expect(accountStateOf("signed_out")).toBe("signed_out");
        for (const s of ["stopped", "starting", "error", undefined] as const) expect(accountStateOf(s)).toBe("unknown");
    });
});

describe("failureVerdict: an account cause follows srv's state", () => {
    it("auth stays active while srv says signed out, or says nothing", () => {
        expect(failureVerdict(fail("auth"), live({ account: "signed_out" }), NOW)).toBe("active");
        expect(failureVerdict(fail("auth"), live({ account: "unknown" }), NOW)).toBe("active");
    });

    it("auth heals once srv says the agent can work, or a turn completed since", () => {
        expect(failureVerdict(fail("auth"), live({ account: "can_work" }), NOW)).toBe("healed");
        expect(failureVerdict(fail("auth"), live({ completedTurnAt: AT + 1 }), NOW)).toBe("healed");
        expect(failureVerdict(fail("auth"), live({ completedTurnAt: AT - 1 }), NOW)).toBe("active");
    });

    it("usage_limit is active while srv says limited and heals when it can work or the reset passes", () => {
        const limit = fail("usage_limit", { resetsAtMs: NOW + 60_000 });
        expect(failureVerdict(limit, live({ account: "limited" }), NOW)).toBe("active");
        expect(failureVerdict(limit, live({ account: "can_work" }), NOW)).toBe("healed");
        expect(failureVerdict(fail("usage_limit", { resetsAtMs: NOW - 1 }), live(), NOW)).toBe("healed");
        expect(failureVerdict(fail("usage_limit"), live({ account: "limited" }), NOW)).toBe("active");
    });

    it("the pre-launch Not signed in row is never judged here", () => {
        const synthetic = { ...fail("auth"), turnAttempted: false };
        expect(failureVerdict(synthetic, live({ account: "can_work" }), NOW)).toBe("active");
    });
});

describe("failureVerdict: a transient cause follows the pane's turns", () => {
    it("is active while the ladder runs, lapsed once it has ended, healed by a completed turn", () => {
        for (const code of ["rate_limited", "overloaded", "network"] as const) {
            expect(failureVerdict(fail(code), live(), NOW)).toBe("active");
            expect(failureVerdict(fail(code), live({ ladderEnded: true }), NOW)).toBe("lapsed");
            expect(failureVerdict(fail(code), live({ ladderEnded: true, completedTurnAt: AT + 5 }), NOW)).toBe("healed");
        }
    });

    it("srv's account state never decides a transient cause", () => {
        for (const account of ["limited", "signed_out", "can_work", "unknown"] as const) {
            expect(failureVerdict(fail("rate_limited"), live({ account }), NOW)).toBe("active");
        }
    });

    it("a network failure heals when the API answers again", () => {
        expect(failureVerdict(fail("network"), live({ apiReachable: true }), NOW)).toBe("healed");
        expect(failureVerdict(fail("network"), live({ apiReachable: false }), NOW)).toBe("active");
        expect(failureVerdict(fail("rate_limited"), live({ apiReachable: true }), NOW)).toBe("active");
    });
});

describe("failureVerdict: any other class", () => {
    it("stays until a turn completes", () => {
        for (const code of ["killed", "no_output", "unknown_non_zero", "max_turns", "context_exceeded", "spawn_failure", "live_elsewhere", "open_in_other_pane", "agent_deleted"] as const) {
            expect(failureVerdict(fail(code), live({ account: "can_work", ladderEnded: true }), NOW)).toBe("active");
            expect(failureVerdict(fail(code), live({ completedTurnAt: AT + 1 }), NOW)).toBe("healed");
        }
    });
});

describe("isAccountCause", () => {
    it("is true for auth and usage_limit only", () => {
        expect(isAccountCause("auth")).toBe(true);
        expect(isAccountCause("usage_limit")).toBe(true);
        for (const code of ["rate_limited", "overloaded", "network", "killed", "spawn_failure"] as const) {
            expect(isAccountCause(code)).toBe(false);
        }
    });
});

describe("agoLabel", () => {
    it("reads minutes, hours and days", () => {
        expect(agoLabel(NOW - 10_000, NOW)).toBe("just now");
        expect(agoLabel(NOW - 12 * 60_000, NOW)).toBe("12 min ago");
        expect(agoLabel(NOW - 3 * 3_600_000, NOW)).toBe("3 h ago");
        expect(agoLabel(NOW - 72 * 3_600_000, NOW)).toBe("3 d ago");
    });
});
