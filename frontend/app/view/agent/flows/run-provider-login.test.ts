// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for runProviderLogin (retro-headless-login-browser-open-2026-07-20,
 * PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md §7).
 *
 * Pins the three-tier fallback that replaced the old "no URL captured ->
 * dead end, go click a different button" behavior of /login and
 * "Login Again": URL-capture, then (Claude only) mint-a-real-account +
 * copy-from-global-login, then a real terminal window polled for the
 * resulting credential — with the same real-account registration on
 * success. "Single point, not global": a seeded credential must land in a
 * real IdentityAccount's own dir, not the shared default one.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    runCliLogin: vi.fn(),
    checkCliAuth: vi.fn(),
    cancelCliLogin: vi.fn(),
    getCliLoginStatus: vi.fn(),
    openPane: vi.fn(),
    seedProviderAuthFromGlobal: vi.fn(),
    openLoginTerminal: vi.fn(),
    ensureAccountDir: vi.fn(),
    upsertIdentityAccount: vi.fn(),
    linkAgentIdentity: vi.fn(),
    setMeta: vi.fn(),
    checkCliAuthCommand: vi.fn(),
}));

vi.mock("@/app/store/global", () => ({
    getApi: () => ({
        runCliLogin: hub.runCliLogin,
        checkCliAuth: hub.checkCliAuth,
        cancelCliLogin: hub.cancelCliLogin,
        getCliLoginStatus: hub.getCliLoginStatus,
        seedProviderAuthFromGlobal: hub.seedProviderAuthFromGlobal,
        openLoginTerminal: hub.openLoginTerminal,
    }),
}));
vi.mock("./open-oauth-pane", () => ({ openOAuthBrowserPane: hub.openPane }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        EnsureAccountDirCommand: (...args: unknown[]) => hub.ensureAccountDir(...args),
        UpsertIdentityAccountCommand: (...args: unknown[]) => hub.upsertIdentityAccount(...args),
        LinkAgentIdentityCommand: (...args: unknown[]) => hub.linkAgentIdentity(...args),
        SetMetaCommand: (...args: unknown[]) => hub.setMeta(...args),
        CheckCliAuthCommand: (...args: unknown[]) => hub.checkCliAuthCommand(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/wos", () => ({ makeORef: (kind: string, id: string) => `${kind}:${id}` }));

import { runProviderLogin } from "./run-provider-login";

const claude = {
    id: "claude",
    authLoginCommand: ["auth", "login"],
    authConfigDirEnvVar: "CLAUDE_CONFIG_DIR",
    authCheckCommand: ["auth", "status"],
    requiresLoginTty: true,
} as any;

const codex = {
    id: "codex",
    authLoginCommand: ["login"],
    authConfigDirEnvVar: "CODEX_HOME",
    authCheckCommand: ["auth", "status"],
} as any;

const MINTED = { accountId: "acct-new", dir: "C:/agentmux/identities/acct-new/claude" };

beforeEach(() => {
    hub.runCliLogin.mockReset();
    hub.checkCliAuth.mockReset();
    hub.cancelCliLogin.mockReset().mockResolvedValue(undefined);
    hub.getCliLoginStatus.mockReset().mockResolvedValue({ active: false, credential_changed: true });
    hub.openPane.mockReset().mockResolvedValue("pane");
    hub.seedProviderAuthFromGlobal.mockReset();
    hub.openLoginTerminal.mockReset().mockResolvedValue({ opened: true });
    hub.ensureAccountDir.mockReset().mockResolvedValue(MINTED);
    hub.upsertIdentityAccount.mockReset().mockResolvedValue({});
    hub.linkAgentIdentity.mockReset().mockResolvedValue(undefined);
    hub.setMeta.mockReset().mockResolvedValue(undefined);
    hub.checkCliAuthCommand.mockReset();
});
afterEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
});

describe("runProviderLogin", () => {
    it("returns 'opened' when tier 1 captures a URL — no fallback tier runs, but the account dir is STILL minted up front and reported via onAccountRegistered (reagent P0 on #2263: a provider whose tier 1 succeeds, e.g. gemini/copilot, must still land in an isolated dir with a real account behind it)", async () => {
        hub.runCliLogin.mockResolvedValue("https://claude.ai/oauth/authorize");
        const onAccountRegistered = vi.fn();

        const outcome = await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            onAccountRegistered,
        });

        expect(outcome).toBe("opened");
        expect(hub.ensureAccountDir).toHaveBeenCalledTimes(1);
        // Tier 1 doesn't confirm completion — so it must NOT persist/link
        // the account itself (nothing has actually logged in yet); it only
        // reports the minted (not-yet-persisted) account to the caller.
        expect(hub.upsertIdentityAccount).not.toHaveBeenCalled();
        expect(onAccountRegistered).toHaveBeenCalledWith(MINTED.accountId, MINTED.dir);
        expect(hub.seedProviderAuthFromGlobal).not.toHaveBeenCalled();
        expect(hub.openLoginTerminal).not.toHaveBeenCalled();
        // Tier 1 succeeded — nothing to cancel, and no fallback tier ran
        // that could race against a still-live tier-1 child.
        expect(hub.cancelCliLogin).not.toHaveBeenCalled();
    });

    it("tier 1 points the login at the minted isolated dir, not whatever authEnv the caller originally passed", async () => {
        hub.runCliLogin.mockResolvedValue("https://claude.ai/oauth/authorize");

        await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/some-other-dir" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
        });

        expect(hub.runCliLogin).toHaveBeenCalledWith(
            "x",
            ["auth", "login"],
            { CLAUDE_CONFIG_DIR: MINTED.dir },
            true,
        );
    });

    it("falls through to tier 2 — mints a real account dir, seeds it, and registers the account — when tier 1 captures no URL, for claude", async () => {
        hub.runCliLogin.mockResolvedValue(null);
        hub.seedProviderAuthFromGlobal.mockResolvedValue({ seeded: true });

        const outcome = await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
        });

        expect(outcome).toBe("seeded");
        expect(hub.ensureAccountDir).toHaveBeenCalledWith({}, { providerId: "claude", existingAccountId: undefined });
        expect(hub.seedProviderAuthFromGlobal).toHaveBeenCalledWith("claude", MINTED.dir);
        expect(hub.upsertIdentityAccount).toHaveBeenCalledWith(
            {},
            expect.objectContaining({ id: MINTED.accountId, provider: "claude", kind: "oauth" }),
        );
        expect(hub.openLoginTerminal).not.toHaveBeenCalled();
        // Tier 1 timed out with no URL — its abandoned CLI child must be
        // cancelled before tier 2 mints/seeds an account.
        expect(hub.cancelCliLogin).toHaveBeenCalledTimes(1);
    });

    it("retries persistSeededAccount once on a transient failure after a successful seed, instead of silently falling through to tier 3 for a login that already succeeded (reagent P2)", async () => {
        hub.runCliLogin.mockResolvedValue(null);
        hub.seedProviderAuthFromGlobal.mockResolvedValue({ seeded: true });
        hub.upsertIdentityAccount
            .mockRejectedValueOnce(new Error("transient RPC error"))
            .mockResolvedValueOnce({});

        const outcome = await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
        });

        expect(outcome).toBe("seeded");
        expect(hub.upsertIdentityAccount).toHaveBeenCalledTimes(2);
        expect(hub.openLoginTerminal).not.toHaveBeenCalled();
    });

    it("falls through to terminal (with a clear error logged) if persistSeededAccount fails on both attempts", async () => {
        hub.runCliLogin.mockResolvedValue(null);
        hub.seedProviderAuthFromGlobal.mockResolvedValue({ seeded: true });
        hub.upsertIdentityAccount.mockRejectedValue(new Error("persistent RPC error"));
        const log = vi.fn();

        const outcome = await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log,
            isCancelled: () => true,
        });

        expect(hub.upsertIdentityAccount).toHaveBeenCalledTimes(2);
        expect(outcome).toBe("terminal-timeout");
        expect(log).toHaveBeenCalledWith(
            "auth",
            expect.stringMatching(/login succeeded, but AgentMux couldn't save the account record/i),
            "error",
        );
    });

    it("tier 2 mints the account dir exactly once (ensureAccountDir called once, not once per tier)", async () => {
        hub.runCliLogin.mockResolvedValue(null);
        hub.seedProviderAuthFromGlobal.mockResolvedValue({ seeded: true });

        await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
        });

        expect(hub.ensureAccountDir).toHaveBeenCalledTimes(1);
    });

    it("when tier 2's seed fails partway (dir minted but not seeded), tier 3 reuses the SAME minted account instead of minting a second one", async () => {
        vi.useFakeTimers();
        hub.runCliLogin.mockResolvedValue(null);
        hub.seedProviderAuthFromGlobal
            .mockResolvedValueOnce({ seeded: false, status: "missing" }) // tier 2: no global login to copy
            .mockResolvedValueOnce({ seeded: true }); // tier 3's first poll: user finished the browser login

        const promise = runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
        });
        await vi.advanceTimersByTimeAsync(5_000);
        const outcome = await promise;

        expect(outcome).toBe("terminal-success");
        // Exactly one mint for the whole call, not one per tier — the same
        // MINTED.accountId/dir is what both tier 2's seed attempt and tier
        // 3's terminal poll operate against.
        expect(hub.ensureAccountDir).toHaveBeenCalledTimes(1);
        expect(hub.upsertIdentityAccount).toHaveBeenCalledTimes(1);
        expect(hub.upsertIdentityAccount).toHaveBeenCalledWith(
            {},
            expect.objectContaining({ id: MINTED.accountId }),
        );
    });

    it("links the new account and updates block meta when a linkTarget is given", async () => {
        hub.runCliLogin.mockResolvedValue(null);
        hub.seedProviderAuthFromGlobal.mockResolvedValue({ seeded: true });

        const outcome = await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            linkTarget: { blockId: "block-1", agentDefinitionId: "def-1" },
        });

        expect(outcome).toBe("seeded");
        expect(hub.linkAgentIdentity).toHaveBeenCalledWith({}, {
            agent_id: "def-1",
            account_id: MINTED.accountId,
            provider: "claude",
        });
        expect(hub.setMeta).toHaveBeenCalledWith({}, {
            oref: "block:block-1",
            meta: { "cmd:env": { CLAUDE_CONFIG_DIR: MINTED.dir } },
        });
    });

    it("skips tier 2 for non-claude providers (host rejects seed-from-global for them) but STILL mints an account dir and points the terminal at it directly (not stripped — codex has no seed-from-global to copy back from)", async () => {
        hub.runCliLogin.mockResolvedValue(null);
        hub.checkCliAuthCommand.mockResolvedValue({ authenticated: false });

        const outcome = await runProviderLogin({
            provider: codex,
            cliPath: "x",
            authEnv: { CODEX_HOME: "C:/auth", OTHER: "keep" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            isCancelled: () => true, // abort tier 3's poll immediately, we only care it was opened
        });

        expect(hub.ensureAccountDir).toHaveBeenCalledTimes(1); // reagent P0: used to skip minting for non-claude entirely
        expect(hub.seedProviderAuthFromGlobal).not.toHaveBeenCalled();
        // The env var is KEPT and points at the minted isolated dir — not
        // stripped the way Claude's tier 3 strips it (codex has no global
        // login to copy back from, so the login must land directly here).
        expect(hub.openLoginTerminal).toHaveBeenCalledWith("x", ["login"], { OTHER: "keep", CODEX_HOME: MINTED.dir });
        expect(outcome).toBe("terminal-timeout");
    });

    it("non-claude tier 3 success: polls CheckCliAuthCommand against the isolated dir (not seedGlobalLogin) and persists the account once authenticated", async () => {
        vi.useFakeTimers();
        hub.runCliLogin.mockResolvedValue(null);
        hub.checkCliAuthCommand
            .mockResolvedValueOnce({ authenticated: false }) // first poll: not yet
            .mockResolvedValueOnce({ authenticated: true }); // second poll: user finished in the terminal

        const promise = runProviderLogin({
            provider: codex,
            cliPath: "x",
            authEnv: { CODEX_HOME: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
        });
        await vi.advanceTimersByTimeAsync(10_000);
        const outcome = await promise;

        expect(outcome).toBe("terminal-success");
        expect(hub.seedProviderAuthFromGlobal).not.toHaveBeenCalled();
        expect(hub.checkCliAuthCommand).toHaveBeenCalledWith(
            {},
            { cli_path: "x", auth_check_args: ["auth", "status"], auth_env: { CODEX_HOME: MINTED.dir } },
            { timeout: 10000 },
        );
        expect(hub.upsertIdentityAccount).toHaveBeenCalledWith(
            {},
            expect.objectContaining({ id: MINTED.accountId, provider: "codex" }),
        );
    });

    it("a non-claude tier-3 login that never authenticates times out cleanly instead of crashing (reagent P0: seedGlobalLogin used to be called unconditionally and threw for non-claude providers)", async () => {
        hub.runCliLogin.mockResolvedValue(null);
        hub.checkCliAuthCommand.mockResolvedValue({ authenticated: false });

        const outcome = await runProviderLogin({
            provider: codex,
            cliPath: "x",
            authEnv: { CODEX_HOME: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            isCancelled: () => true,
        });

        expect(outcome).toBe("terminal-timeout");
        expect(hub.seedProviderAuthFromGlobal).not.toHaveBeenCalled();
        expect(hub.upsertIdentityAccount).not.toHaveBeenCalled();
    });

    it("falls through to tier 3 (real terminal), mints an account dir up front, and registers it once the login lands on disk", async () => {
        vi.useFakeTimers();
        hub.runCliLogin.mockResolvedValue(null);
        hub.seedProviderAuthFromGlobal
            .mockResolvedValueOnce({ seeded: false, status: "missing" }) // tier 2: no global login yet
            .mockResolvedValueOnce({ seeded: true }); // first tier-3 poll: user finished the browser login

        const promise = runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
        });
        await vi.advanceTimersByTimeAsync(5_000);
        const outcome = await promise;

        expect(outcome).toBe("terminal-success");
        // Tier 3's terminal env is stripped of the config-dir var so the
        // fresh login lands in the user's global dir, not the minted one.
        expect(hub.openLoginTerminal).toHaveBeenCalledWith("x", ["auth", "login"], {});
        expect(hub.seedProviderAuthFromGlobal).toHaveBeenLastCalledWith("claude", MINTED.dir);
        expect(hub.upsertIdentityAccount).toHaveBeenCalledWith(
            {},
            expect.objectContaining({ id: MINTED.accountId, provider: "claude" }),
        );
    });

    it("returns 'terminal-timeout' without a full 5-minute wait when cancelled", async () => {
        hub.runCliLogin.mockResolvedValue(null);
        hub.seedProviderAuthFromGlobal.mockResolvedValue({ seeded: false, status: "missing" });

        const outcome = await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            isCancelled: () => true,
        });

        expect(outcome).toBe("terminal-timeout");
        expect(hub.upsertIdentityAccount).not.toHaveBeenCalled();
    });

    it("returns 'terminal-unavailable' when the terminal itself can't be opened (e.g. unsupported platform)", async () => {
        hub.runCliLogin.mockResolvedValue(null);
        hub.seedProviderAuthFromGlobal.mockResolvedValue({ seeded: false, status: "missing" });
        hub.openLoginTerminal.mockRejectedValue(new Error("open_login_terminal: not yet implemented on this platform"));

        const log = vi.fn();
        const outcome = await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log,
        });

        expect(outcome).toBe("terminal-unavailable");
        expect(log).toHaveBeenCalledWith("auth", expect.stringMatching(/couldn't open a terminal/i), "error");
    });

    it("skipTier1 skips the headless URL-capture attempt entirely and goes straight to tier 2", async () => {
        hub.seedProviderAuthFromGlobal.mockResolvedValue({ seeded: true });

        const outcome = await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            skipTier1: true,
        });

        expect(outcome).toBe("seeded");
        expect(hub.runCliLogin).not.toHaveBeenCalled();
        expect(hub.openPane).not.toHaveBeenCalled();
    });

    it("reagent P1: fires onTierChange({tier: 'fallback'}) once tier 1 fails, so a caller's stale URL-capture countdown doesn't freeze while tier 2/3 (up to 5 more minutes) take over", async () => {
        hub.runCliLogin.mockResolvedValue(null); // tier 1: no URL captured
        hub.seedProviderAuthFromGlobal.mockResolvedValue({ seeded: true }); // tier 2 succeeds fast
        const onTierChange = vi.fn();

        const outcome = await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            onTierChange,
        });

        expect(outcome).toBe("seeded");
        expect(onTierChange).toHaveBeenCalledWith({ tier: "fallback" });
        // Tier 2 resolved before any terminal opened — no "polling" event yet.
        expect(onTierChange).not.toHaveBeenCalledWith(expect.objectContaining({ tier: "polling" }));
    });

    it("reagent P1: fires onTierChange({tier: 'polling', deadlineMs}) once the terminal actually opens, with a live ~5-minute deadline matching the real poll timeout", async () => {
        vi.useFakeTimers();
        const before = Date.now();
        hub.runCliLogin.mockResolvedValue(null);
        hub.seedProviderAuthFromGlobal
            .mockResolvedValueOnce({ seeded: false, status: "missing" }) // tier 2: no global login yet
            .mockResolvedValueOnce({ seeded: true }); // first tier-3 poll succeeds
        const onTierChange = vi.fn();

        const promise = runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            onTierChange,
        });
        await vi.advanceTimersByTimeAsync(5_000);
        await promise;

        expect(onTierChange).toHaveBeenCalledWith({ tier: "fallback" });
        const pollingCall = onTierChange.mock.calls.find((c) => c[0].tier === "polling");
        expect(pollingCall).toBeDefined();
        const { deadlineMs } = pollingCall![0];
        // Matches pollForGlobalLoginSeed's own default timeoutMs (5 min) —
        // if that default ever changes, this assertion should move with it.
        expect(deadlineMs).toBeGreaterThanOrEqual(before + 5 * 60 * 1000 - 1000);
        expect(deadlineMs).toBeLessThanOrEqual(before + 5 * 60 * 1000 + 1000);
    });

    it("existingAccountId is threaded through to tier 2's account minting — reconnects the SAME account instead of minting a new one (reagent P1: retries were orphaning a new account every time)", async () => {
        hub.runCliLogin.mockResolvedValue(null);
        hub.seedProviderAuthFromGlobal.mockResolvedValue({ seeded: true });

        await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            existingAccountId: "acct-existing",
        });

        expect(hub.ensureAccountDir).toHaveBeenCalledWith(
            {},
            { providerId: "claude", existingAccountId: "acct-existing" },
        );
    });

    it("existingAccountId is threaded through to tier 3's account minting too", async () => {
        vi.useFakeTimers();
        hub.runCliLogin.mockResolvedValue(null);
        hub.seedProviderAuthFromGlobal
            .mockResolvedValueOnce({ seeded: false, status: "missing" })
            .mockResolvedValueOnce({ seeded: true });

        const promise = runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            existingAccountId: "acct-existing",
        });
        await vi.advanceTimersByTimeAsync(5_000);
        await promise;

        expect(hub.ensureAccountDir).toHaveBeenCalledWith(
            {},
            { providerId: "claude", existingAccountId: "acct-existing" },
        );
    });

    it("onAccountRegistered fires with the account id + dir on tier 2 success — before finalizeAccount, so a caller can rebuild its own authEnv to recheck the NEW dir (reagent P0: a caller's stale authEnv otherwise reports authenticated:false right after a successful login)", async () => {
        hub.runCliLogin.mockResolvedValue(null);
        hub.seedProviderAuthFromGlobal.mockResolvedValue({ seeded: true });
        const onAccountRegistered = vi.fn();

        const outcome = await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            onAccountRegistered,
        });

        expect(outcome).toBe("seeded");
        expect(onAccountRegistered).toHaveBeenCalledWith(MINTED.accountId, MINTED.dir);
    });

    it("onAccountRegistered fires on tier 3 success too, and NOT at all if persistSeededAccount fails", async () => {
        vi.useFakeTimers();
        hub.runCliLogin.mockResolvedValue(null);
        hub.seedProviderAuthFromGlobal
            .mockResolvedValueOnce({ seeded: false, status: "missing" })
            .mockResolvedValueOnce({ seeded: true });
        const onAccountRegistered = vi.fn();

        const promise = runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            onAccountRegistered,
        });
        await vi.advanceTimersByTimeAsync(5_000);
        await promise;

        expect(onAccountRegistered).toHaveBeenCalledWith(MINTED.accountId, MINTED.dir);

        onAccountRegistered.mockClear();
        // Rejects on BOTH attempts — tier 3 retries once (reagent P2,
        // matching tier 2's identical safety net), so a single-rejection
        // mock would now succeed on the retry and fire onAccountRegistered.
        hub.upsertIdentityAccount.mockRejectedValue(new Error("db error"));
        hub.seedProviderAuthFromGlobal
            .mockReset()
            .mockResolvedValueOnce({ seeded: false, status: "missing" })
            .mockResolvedValueOnce({ seeded: true });

        const promise2 = runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            onAccountRegistered,
        });
        await vi.advanceTimersByTimeAsync(5_000);
        await promise2;
        expect(onAccountRegistered).not.toHaveBeenCalled();
    });

    it("retries persistSeededAccount once on tier 3 too, on a transient failure after terminal-success is detected (reagent P2, mirrors tier 2's identical retry)", async () => {
        vi.useFakeTimers();
        hub.runCliLogin.mockResolvedValue(null);
        hub.seedProviderAuthFromGlobal
            .mockResolvedValueOnce({ seeded: false, status: "missing" })
            .mockResolvedValueOnce({ seeded: true });
        hub.upsertIdentityAccount
            .mockRejectedValueOnce(new Error("transient RPC error"))
            .mockResolvedValueOnce({});
        const onAccountRegistered = vi.fn();

        const promise = runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            onAccountRegistered,
        });
        await vi.advanceTimersByTimeAsync(5_000);
        const outcome = await promise;

        expect(outcome).toBe("terminal-success");
        expect(hub.upsertIdentityAccount).toHaveBeenCalledTimes(2);
        expect(onAccountRegistered).toHaveBeenCalledWith(MINTED.accountId, MINTED.dir);
    });

    it("onAccountRegistered fires on non-claude tier 3 success too", async () => {
        vi.useFakeTimers();
        hub.runCliLogin.mockResolvedValue(null);
        hub.checkCliAuthCommand
            .mockResolvedValueOnce({ authenticated: false })
            .mockResolvedValueOnce({ authenticated: true });
        const onAccountRegistered = vi.fn();

        const promise = runProviderLogin({
            provider: codex,
            cliPath: "x",
            authEnv: { CODEX_HOME: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            onAccountRegistered,
        });
        await vi.advanceTimersByTimeAsync(10_000);
        await promise;

        expect(onAccountRegistered).toHaveBeenCalledWith(MINTED.accountId, MINTED.dir);
    });
});

// The awaited in-app login session (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md
// §3.1): with `awaitTier1Completion`, a tier-1 URL capture doesn't return
// "opened" — the call stays alive as the session, polling for the login child
// exiting AND credential material landing in the minted isolated dir, then
// persists+links the account itself and resolves "inapp-success"/
// "inapp-timeout". The URL the pinned Claude CLI (2.1.198+) actually prints is
// used throughout; host-side capture of that URL (incl. OSC-8-wrapped) is
// pinned separately in cli_login.rs's extract_url_claude_authorize_tests.
describe("runProviderLogin — awaited in-app session (awaitTier1Completion)", () => {
    const CLAUDE_AUTHORIZE_URL =
        "https://claude.com/cai/oauth/authorize?code=true&client_id=abc&code_challenge=xyz&state=st";

    it("returns 'inapp-success' once the login child has exited and the credential is present — persists and links the account itself, reaps the child, and never touches tiers 2/3", async () => {
        vi.useFakeTimers();
        hub.runCliLogin.mockResolvedValue(CLAUDE_AUTHORIZE_URL);
        hub.getCliLoginStatus.mockResolvedValue({ active: false, credential_changed: true }); // child exited, credential written
        hub.checkCliAuthCommand.mockResolvedValue({ authenticated: true }); // credential landed
        const onAccountRegistered = vi.fn();
        const setAuthUrl = vi.fn();

        const promise = runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl,
            log: vi.fn(),
            awaitTier1Completion: true,
            onAccountRegistered,
            linkTarget: { blockId: "block-1", agentDefinitionId: "def-1" },
        });
        await vi.advanceTimersByTimeAsync(2_000);
        const outcome = await promise;

        expect(outcome).toBe("inapp-success");
        // The captured URL was surfaced for the caller's auth-url UI.
        expect(setAuthUrl).toHaveBeenCalledWith(CLAUDE_AUTHORIZE_URL);
        // The completion poll asked the CLI about the MINTED isolated dir.
        expect(hub.checkCliAuthCommand).toHaveBeenCalledWith(
            {},
            { cli_path: "x", auth_check_args: ["auth", "status"], auth_env: { CLAUDE_CONFIG_DIR: MINTED.dir } },
            { timeout: 10000 },
        );
        // Persisted + linked HERE (unlike the legacy "opened" contract,
        // where the caller does this via persistAndLinkAccount).
        expect(hub.upsertIdentityAccount).toHaveBeenCalledWith(
            {},
            expect.objectContaining({ id: MINTED.accountId, provider: "claude", kind: "oauth" }),
        );
        expect(hub.linkAgentIdentity).toHaveBeenCalledWith({}, {
            agent_id: "def-1",
            account_id: MINTED.accountId,
            provider: "claude",
        });
        expect(onAccountRegistered).toHaveBeenCalledWith(MINTED.accountId, MINTED.dir);
        // Child reaped on the way out (idempotent host call), and no
        // fallback tier ever ran.
        expect(hub.cancelCliLogin).toHaveBeenCalledTimes(1);
        expect(hub.seedProviderAuthFromGlobal).not.toHaveBeenCalled();
        expect(hub.openLoginTerminal).not.toHaveBeenCalled();
    });

    it("completion requires the CHILD EXIT, not just a positive credential probe — a reconnect into an existing dir whose stale token reports 'authenticated' must NOT complete (or reap the child) while the login is still running", async () => {
        vi.useFakeTimers();
        hub.runCliLogin.mockResolvedValue(CLAUDE_AUTHORIZE_URL);
        // Stale-credential reconnect: the auth probe says "authenticated"
        // from tick 1 (present-but-expired token — force-login.ts's
        // documented false positive), but the login child is still alive
        // for two ticks before finishing.
        hub.checkCliAuthCommand.mockResolvedValue({ authenticated: true });
        hub.getCliLoginStatus
            .mockResolvedValueOnce({ active: true, credential_changed: true })
            .mockResolvedValueOnce({ active: true, credential_changed: true })
            .mockResolvedValueOnce({ active: false, credential_changed: true });

        const promise = runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            awaitTier1Completion: true,
            existingAccountId: "acct-existing",
        });
        await vi.advanceTimersByTimeAsync(6_000);
        const outcome = await promise;

        expect(outcome).toBe("inapp-success");
        // Three status probes = it genuinely waited for the child to exit
        // instead of trusting the first "authenticated" tick.
        expect(hub.getCliLoginStatus).toHaveBeenCalledTimes(3);
    });

    it("reagent P1 on PR #2410: a reconnect whose child crashes instantly must NOT report success off the OLD stale-but-still-shaped credential — completion also requires credential_changed", async () => {
        vi.useFakeTimers();
        hub.runCliLogin.mockResolvedValue(CLAUDE_AUTHORIZE_URL);
        // The reconnect's stale token already on disk BEFORE this attempt
        // started reports "authenticated" throughout — present-but-expired
        // tokens don't fail this local presence check (force-login.ts's
        // documented false positive) — but the child dies on tick 1 without
        // ever touching the credential file, so credential_changed stays
        // false the whole time.
        hub.checkCliAuthCommand.mockResolvedValue({ authenticated: true });
        hub.getCliLoginStatus.mockResolvedValue({ active: false, credential_changed: false });

        const promise = runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            awaitTier1Completion: true,
            existingAccountId: "acct-existing",
        });
        // 2 grace re-checks after the first child-gone observation, same
        // window as the "no credential ever landing" case.
        await vi.advanceTimersByTimeAsync(6_000);
        const outcome = await promise;

        expect(outcome).toBe("inapp-timeout");
        expect(hub.upsertIdentityAccount).not.toHaveBeenCalled();
    });

    it("fails fast with 'inapp-timeout' when the child exits WITHOUT a credential ever landing (login crashed/failed) — after grace re-checks, well before the full window", async () => {
        vi.useFakeTimers();
        hub.runCliLogin.mockResolvedValue(CLAUDE_AUTHORIZE_URL);
        hub.getCliLoginStatus.mockResolvedValue({ active: false, credential_changed: false });
        hub.checkCliAuthCommand.mockResolvedValue({ authenticated: false });

        const promise = runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            awaitTier1Completion: true,
        });
        // 2 grace re-checks after the first child-gone observation = 3
        // poll ticks (6s), nowhere near the 5-minute window.
        await vi.advanceTimersByTimeAsync(6_000);
        const outcome = await promise;

        expect(outcome).toBe("inapp-timeout");
        expect(hub.upsertIdentityAccount).not.toHaveBeenCalled();
        expect(hub.cancelCliLogin).toHaveBeenCalledTimes(1);
        // No automatic tier 2/3 fallback — the user already has the URL.
        expect(hub.seedProviderAuthFromGlobal).not.toHaveBeenCalled();
        expect(hub.openLoginTerminal).not.toHaveBeenCalled();
    });

    it("returns 'inapp-timeout' promptly when cancelled, reaping the child", async () => {
        hub.runCliLogin.mockResolvedValue(CLAUDE_AUTHORIZE_URL);

        const outcome = await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            awaitTier1Completion: true,
            isCancelled: () => true,
        });

        expect(outcome).toBe("inapp-timeout");
        expect(hub.upsertIdentityAccount).not.toHaveBeenCalled();
        expect(hub.cancelCliLogin).toHaveBeenCalledTimes(1);
    });

    it("feature-gate: when NO URL is captured within the window (older CLI, ≤2.1.183 behavior), falls through to tier 2/3 exactly as without the flag — the in-app wait never starts", async () => {
        hub.runCliLogin.mockResolvedValue(null);
        hub.seedProviderAuthFromGlobal.mockResolvedValue({ seeded: true });

        const outcome = await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            awaitTier1Completion: true,
        });

        expect(outcome).toBe("seeded");
        expect(hub.getCliLoginStatus).not.toHaveBeenCalled();
        expect(hub.seedProviderAuthFromGlobal).toHaveBeenCalledWith("claude", MINTED.dir);
    });

    it("fires onTierChange({tier:'inapp-waiting', deadlineMs}) when the awaited session starts, so a caller's phase line can show a live deadline", async () => {
        vi.useFakeTimers();
        const before = Date.now();
        hub.runCliLogin.mockResolvedValue(CLAUDE_AUTHORIZE_URL);
        hub.getCliLoginStatus.mockResolvedValue({ active: false, credential_changed: true });
        hub.checkCliAuthCommand.mockResolvedValue({ authenticated: true });
        const onTierChange = vi.fn();

        const promise = runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            awaitTier1Completion: true,
            onTierChange,
        });
        await vi.advanceTimersByTimeAsync(2_000);
        await promise;

        const call = onTierChange.mock.calls.find((c) => c[0].tier === "inapp-waiting");
        expect(call).toBeDefined();
        const { deadlineMs } = call![0];
        expect(deadlineMs).toBeGreaterThanOrEqual(before + 5 * 60 * 1000 - 1000);
        expect(deadlineMs).toBeLessThanOrEqual(before + 5 * 60 * 1000 + 1000);
    });

    it("persist failure on both attempts still resolves 'inapp-success' (credential is genuinely on disk) but does NOT fire onAccountRegistered — callers gate their success messaging on that, same as the terminal-success contract", async () => {
        vi.useFakeTimers();
        hub.runCliLogin.mockResolvedValue(CLAUDE_AUTHORIZE_URL);
        hub.getCliLoginStatus.mockResolvedValue({ active: false, credential_changed: true });
        hub.checkCliAuthCommand.mockResolvedValue({ authenticated: true });
        hub.upsertIdentityAccount.mockRejectedValue(new Error("db error"));
        const onAccountRegistered = vi.fn();
        const log = vi.fn();

        const promise = runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log,
            awaitTier1Completion: true,
            onAccountRegistered,
        });
        await vi.advanceTimersByTimeAsync(2_000);
        const outcome = await promise;

        expect(outcome).toBe("inapp-success");
        expect(hub.upsertIdentityAccount).toHaveBeenCalledTimes(2); // one retry
        expect(onAccountRegistered).not.toHaveBeenCalled();
        expect(log).toHaveBeenCalledWith(
            "auth",
            expect.stringMatching(/couldn't save the account record/i),
            "error",
        );
    });

    it("downgrades to the legacy 'opened' contract when the account-dir mint failed — no isolated dir to poll or persist against", async () => {
        hub.runCliLogin.mockResolvedValue(CLAUDE_AUTHORIZE_URL);
        hub.ensureAccountDir.mockResolvedValue(null);

        const outcome = await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            awaitTier1Completion: true,
        });

        expect(outcome).toBe("opened");
        expect(hub.getCliLoginStatus).not.toHaveBeenCalled();
        expect(hub.upsertIdentityAccount).not.toHaveBeenCalled();
    });

    it("codex P2 on PR #2410: when a DIFFERENT surface's login supersedes this one mid-poll (host generation advances), does not call cancelCliLogin — that would kill the newer, unrelated login instead of reaping this one", async () => {
        vi.useFakeTimers();
        hub.runCliLogin.mockResolvedValue(CLAUDE_AUTHORIZE_URL);
        hub.checkCliAuthCommand.mockResolvedValue({ authenticated: false });
        hub.getCliLoginStatus
            // First read establishes this poll's own baseline generation.
            .mockResolvedValueOnce({ active: true, credential_changed: true, generation: 5 })
            // A different surface starts a new login — host generation
            // advances. From here on, active/credential_changed describe
            // THAT newer child, not this poll's own.
            .mockResolvedValueOnce({ active: false, credential_changed: true, generation: 6 });

        const promise = runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
            awaitTier1Completion: true,
        });
        await vi.advanceTimersByTimeAsync(4_000);
        const outcome = await promise;

        expect(outcome).toBe("inapp-timeout");
        expect(hub.upsertIdentityAccount).not.toHaveBeenCalled();
        // The whole point: must NOT reap a login that isn't this attempt's.
        expect(hub.cancelCliLogin).not.toHaveBeenCalled();
    });
});
