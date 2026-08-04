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
        forceControllerRefresh: vi.fn().mockResolvedValue(true),
        isTurnActive: () => false,
        deferControllerRefreshUntilIdle: vi.fn(),
        beginRecoveryFlow: vi.fn(),
        endRecoveryFlow: vi.fn(),
        isCancelled: () => false,
        resetCancelled: vi.fn(),
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

describe("/login runs the in-app tier 1 for Claude (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.2)", () => {
    it("passes skipTier1: false for a provider without headlessLoginUrlUnsupported — Claude's flag was dropped, so its URL-capture tier runs and the AuthUrlBox paste flow is reachable from /login", async () => {
        vi.useFakeTimers();
        hub.persistAndLinkAccount.mockResolvedValue(true);
        const ctx = makeCtx();

        const promise = loginCommand.handler(ctx, "");
        await vi.advanceTimersByTimeAsync(2_000);
        await promise;

        expect(hub.runProviderLogin).toHaveBeenCalledWith(
            expect.objectContaining({ skipTier1: false }),
        );
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

describe("reagent P1 on PR #2413 (round 3, second pass): /login's poll notices an external cancel", () => {
    // The AuthUrlBox Cancel / "Use terminal instead" buttons call
    // useAgentControllerStatus's cancelLogin()/useTerminalInstead() directly,
    // not through this handler — ctx.isCancelled() is how the poll below
    // learns that happened instead of running to its own 5-minute timeout,
    // long past useTerminalInstead()'s 20s backstop.
    it("stops polling and returns a silent ok as soon as isCancelled() flips true, instead of running to the 5-minute timeout", async () => {
        vi.useFakeTimers();
        hub.checkCliAuth.mockReset().mockResolvedValue({ authenticated: false });
        const ctx = makeCtx();
        let cancelled = false;
        ctx.isCancelled = () => cancelled;

        const promise = loginCommand.handler(ctx, "");
        await vi.advanceTimersByTimeAsync(6_000);
        cancelled = true;
        await vi.advanceTimersByTimeAsync(2_000);
        const result = await promise;

        expect(result).toEqual({ kind: "ok" });
        // Not the "no login detected" error — the user explicitly left this
        // flow, so there's nothing wrong to report.
        expect(ctx.log).not.toHaveBeenCalledWith(
            "auth",
            expect.stringMatching(/no login was detected/i),
            expect.anything(),
        );
        expect(ctx.endRecoveryFlow).toHaveBeenCalledOnce();
    });

    it("does not stop polling on its own — still runs to the 5-minute timeout when isCancelled() never flips true", async () => {
        vi.useFakeTimers();
        hub.checkCliAuth.mockReset().mockResolvedValue({ authenticated: false });
        const ctx = makeCtx();

        const promise = loginCommand.handler(ctx, "");
        await vi.advanceTimersByTimeAsync(5 * 60 * 1000);
        const result = await promise;

        expect(result.kind).toBe("error");
    });
});

describe("reagent P1 on PR #2413 (round 3, third pass): /login resets a stale cancellation flag left by an earlier, unrelated attempt", () => {
    it("calls ctx.resetCancelled() before starting", async () => {
        hub.checkCliAuth.mockReset().mockResolvedValue({ authenticated: true });
        const ctx = makeCtx();
        ctx.resetCancelled = vi.fn();

        await loginCommand.handler(ctx, "");

        expect(ctx.resetCancelled).toHaveBeenCalledOnce();
    });

    it("a stale isCancelled()===true left by a DIFFERENT, earlier cancelled attempt does not short-circuit this fresh /login into an unearned 'ok' — resetCancelled() actually clears it, so the poll runs for real", async () => {
        vi.useFakeTimers();
        hub.checkCliAuth.mockReset().mockResolvedValue({ authenticated: false });
        const ctx = makeCtx();
        // Mirrors useAgentControllerStatus's real shared boolean: `true`
        // left over from some earlier, unrelated attempt whose Cancel
        // button fired (e.g. via relogin()'s AuthUrlBox) — /login never
        // touched it before this fix, so it stayed true forever.
        let cancelled = true;
        ctx.isCancelled = () => cancelled;
        ctx.resetCancelled = () => { cancelled = false; };

        const promise = loginCommand.handler(ctx, "");
        // Without resetCancelled() actually clearing the stale flag, the
        // poll's isCancelled() check would already read true on its very
        // first tick and resolve "ok" almost instantly instead of running.
        await vi.advanceTimersByTimeAsync(5 * 60 * 1000);
        const result = await promise;

        expect(result).toEqual({
            kind: "error",
            message:
                "/login: opened a login page, but no login was detected within 5 minutes. " +
                "Complete the login there, then run /login again.",
        });
    });
});

// Codex P1 on PR #2338 (tenth re-review): agentmux-srv's resync_controller
// with force:true unconditionally stops the existing controller process —
// calling forceControllerRefresh while a turn is actively streaming on that
// controller would kill it and discard in-progress work.
describe("/login does not restart an actively-streaming controller (codex P1 on PR #2338, tenth re-review)", () => {
    it("defers forceControllerRefresh (does not skip-and-declare-healthy) when a turn is active", async () => {
        // Codex P1 on PR #2338 (thirteenth re-review): persistent providers
        // keep the controller alive across MANY turns, not just this one —
        // declaring the pane healthy here (as an earlier version of this
        // fix did) would leave the controller on the stale credential
        // indefinitely, every fast-fail guard cleared, until the pane is
        // manually reopened.
        vi.useFakeTimers();
        hub.persistAndLinkAccount.mockResolvedValue(true);
        const ctx = makeCtx();
        ctx.isTurnActive = () => true;

        const promise = loginCommand.handler(ctx, "");
        await vi.advanceTimersByTimeAsync(2_000);
        const result = await promise;

        expect(result).toEqual({ kind: "ok" });
        expect(ctx.forceControllerRefresh).not.toHaveBeenCalled();
        expect(ctx.deferControllerRefreshUntilIdle).toHaveBeenCalledOnce();
        // Guards stay up until the deferred refresh actually runs (once the
        // turn ends) and succeeds — not declared healthy prematurely.
        expect(ctx.notifyControllerHealthy).not.toHaveBeenCalled();
        expect(ctx.clearAuthFailure).not.toHaveBeenCalled();
    });

    it("calls forceControllerRefresh normally when no turn is active", async () => {
        vi.useFakeTimers();
        hub.persistAndLinkAccount.mockResolvedValue(true);
        const ctx = makeCtx();
        ctx.isTurnActive = () => false;

        const promise = loginCommand.handler(ctx, "");
        await vi.advanceTimersByTimeAsync(2_000);
        await promise;

        expect(ctx.forceControllerRefresh).toHaveBeenCalledOnce();
    });
});

// Codex P1 on PR #2338 (tenth re-review): forceControllerRefresh swallows
// its own RPC failures internally (logs a warning, resolves normally) — the
// caller must consume its boolean return, or a failed refresh still gets
// declared "healthy" while the controller stays on the stale credential,
// clearing every fast-fail guard this PR added for nothing.
describe("/login retains auth gating when the controller refresh itself fails (codex P1 on PR #2338, tenth re-review)", () => {
    it("returns an error and does NOT call notifyControllerHealthy/clearAuthFailure when forceControllerRefresh resolves false", async () => {
        vi.useFakeTimers();
        hub.persistAndLinkAccount.mockResolvedValue(true);
        const ctx = makeCtx();
        ctx.isTurnActive = () => false;
        (ctx.forceControllerRefresh as any).mockResolvedValue(false);

        const promise = loginCommand.handler(ctx, "");
        await vi.advanceTimersByTimeAsync(2_000);
        const result = await promise;

        expect(result.kind).toBe("error");
        expect(ctx.notifyControllerHealthy).not.toHaveBeenCalled();
        expect(ctx.clearAuthFailure).not.toHaveBeenCalled();
    });
});
