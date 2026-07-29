// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * reagent P1 (re-review of PR #2318): the /login command's tier-1 "opened"
 * branch discarded persistAndLinkAccount's boolean return and reported
 * success unconditionally — the exact same gap found and fixed in
 * useAgentControllerStatus.ts's relogin() "opened" branch, but this
 * parallel path was never updated. A DB-write failure here must surface as
 * an error, not "login complete", since the resolver's spawn gate will
 * still block the very next turn with no real account behind it.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    checkCliAuth: vi.fn(),
    runProviderLogin: vi.fn(),
    persistAndLinkAccount: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        CheckCliAuthCommand: (...args: unknown[]) => hub.checkCliAuth(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("../../flows/run-provider-login", () => ({
    runProviderLogin: (...args: unknown[]) => hub.runProviderLogin(...args),
    persistAndLinkAccount: (...args: unknown[]) => hub.persistAndLinkAccount(...args),
}));

import { loginCommand } from "./login";
import type { SlashCommandContext } from "../types";

const claude = {
    id: "claude",
    displayName: "Claude",
    authCheckCommand: ["auth", "status"],
    headlessLoginUrlUnsupported: false,
} as any;

function makeCtx(): SlashCommandContext {
    return {
        blockId: "block-1",
        provider: () => claude,
        block: () => ({ meta: { cmd: "claude-cli", agentId: "agent-1" } }),
        documentAtom: [() => [], vi.fn()] as any,
        log: vi.fn(),
        setAuthUrl: vi.fn(),
        notifyControllerHealthy: vi.fn(),
        clearAuthFailure: vi.fn(),
        forceControllerRefresh: vi.fn().mockResolvedValue(undefined),
        beginRecoveryFlow: vi.fn(),
        endRecoveryFlow: vi.fn(),
        openPicker: vi.fn(),
        openHelp: vi.fn(),
    };
}

beforeEach(() => {
    hub.checkCliAuth.mockReset().mockResolvedValue({ authenticated: true });
    // Simulates tier 1: onAccountRegistered fires with a minted (not yet
    // persisted) account, then the outcome resolves "opened" — the handler's
    // own poll loop below is what confirms completion and triggers persist.
    hub.runProviderLogin.mockReset().mockImplementation(async (opts: any) => {
        opts.onAccountRegistered?.("acct-1", "/tmp/acct-1-dir");
        return "opened";
    });
    hub.persistAndLinkAccount.mockReset();
});
afterEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
});

describe("/login — tier-1 'opened' branch persist-failure gating", () => {
    it("returns ok and logs success when persistAndLinkAccount actually persists the account", async () => {
        vi.useFakeTimers();
        hub.persistAndLinkAccount.mockResolvedValue(true);
        const ctx = makeCtx();

        const promise = loginCommand.handler(ctx, "");
        await vi.advanceTimersByTimeAsync(2_000);
        const result = await promise;

        expect(result).toEqual({ kind: "ok" });
        expect(ctx.log).toHaveBeenCalledWith("auth", "login complete — run /cost to verify");
        // Codex P1 on PR #2338: a pane already showing the mount-time
        // "Log in" bar before the user typed /login directly must not have
        // every subsequent message fast-failed forever just because /login
        // bypassed relogin() (the only other path that manages canRetry).
        expect(ctx.notifyControllerHealthy).toHaveBeenCalledOnce();
        // reagent P1 (re-review): a stale pre-existing "auth" failure row
        // must also be cleared, or the caller's next send re-captures it and
        // reproduces the exact "message silently rejected, stale banner
        // reappears" bug this PR exists to fix.
        expect(ctx.clearAuthFailure).toHaveBeenCalledOnce();
        // Codex P1 (seventh re-review): an already-running persistent
        // controller must be restarted onto the new credential, or the next
        // message bypasses every guard in this file and still reaches the
        // stale process.
        expect(ctx.forceControllerRefresh).toHaveBeenCalledOnce();
        // Codex P1 (ninth re-review): begin/end must pair exactly once
        // regardless of outcome — see the dedicated describe block below
        // for the failure-path case that actually motivated this.
        expect(ctx.beginRecoveryFlow).toHaveBeenCalledOnce();
        expect(ctx.endRecoveryFlow).toHaveBeenCalledOnce();
    });

    it("reagent P1 (re-review of PR #2318): returns an error, not ok, when persistAndLinkAccount fails to save the account", async () => {
        vi.useFakeTimers();
        hub.persistAndLinkAccount.mockResolvedValue(false);
        const ctx = makeCtx();

        const promise = loginCommand.handler(ctx, "");
        await vi.advanceTimersByTimeAsync(2_000);
        const result = await promise;

        expect(result).toEqual({
            kind: "error",
            message: "/login: the login succeeded, but AgentMux couldn't save the account record. Try again in a moment.",
        });
        expect(ctx.log).not.toHaveBeenCalledWith("auth", "login complete — run /cost to verify");
        // A persist failure is not a confirmed-healthy credential — the
        // mount-time "Log in" bar (if showing) must stay up.
        expect(ctx.notifyControllerHealthy).not.toHaveBeenCalled();
        expect(ctx.clearAuthFailure).not.toHaveBeenCalled();
        expect(ctx.forceControllerRefresh).not.toHaveBeenCalled();
        expect(ctx.beginRecoveryFlow).toHaveBeenCalledOnce();
        expect(ctx.endRecoveryFlow).toHaveBeenCalledOnce();
    });
});

describe("/login registers as an in-flight recovery (codex P1 on PR #2338, ninth re-review)", () => {
    // Without this, loginWaiting() reads false for the whole duration of a
    // /login attempt — a second message sent while it's still polling gets
    // held with authWasKnownBadAtQueueTime: false (mid-turn "auth" failures
    // never set canRetry either), so a /login that ultimately times out
    // flushes that held message straight to the still-known-bad controller.
    it("calls beginRecoveryFlow before the poll starts, and endRecoveryFlow exactly once even when the login times out", async () => {
        vi.useFakeTimers();
        hub.checkCliAuth.mockReset().mockResolvedValue({ authenticated: false });
        const ctx = makeCtx();

        const promise = loginCommand.handler(ctx, "");
        // Registered synchronously, before the 5-minute poll loop even
        // starts — a message sent the instant after /login is submitted
        // must already see loginWaiting() as true.
        expect(ctx.beginRecoveryFlow).toHaveBeenCalledOnce();
        expect(ctx.endRecoveryFlow).not.toHaveBeenCalled();

        await vi.advanceTimersByTimeAsync(5 * 60 * 1000);
        const result = await promise;

        expect(result).toEqual({
            kind: "error",
            message:
                "/login: opened a login page, but no login was detected within 5 minutes. " +
                "Complete the login there, then run /login again.",
        });
        // The failure path must release the flag exactly as reliably as
        // success — a leaked true here would wedge every future send behind
        // "wait for the login attempt to finish" forever.
        expect(ctx.endRecoveryFlow).toHaveBeenCalledOnce();
    });
});
