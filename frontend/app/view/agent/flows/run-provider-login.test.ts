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
    it("returns 'opened' when tier 1 captures a URL — no fallback tier runs", async () => {
        hub.runCliLogin.mockResolvedValue("https://claude.ai/oauth/authorize");

        const outcome = await runProviderLogin({
            provider: claude,
            cliPath: "x",
            authEnv: { CLAUDE_CONFIG_DIR: "C:/auth" },
            setAuthUrl: vi.fn(),
            log: vi.fn(),
        });

        expect(outcome).toBe("opened");
        expect(hub.ensureAccountDir).not.toHaveBeenCalled();
        expect(hub.seedProviderAuthFromGlobal).not.toHaveBeenCalled();
        expect(hub.openLoginTerminal).not.toHaveBeenCalled();
        // Tier 1 succeeded — nothing to cancel, and no fallback tier ran
        // that could race against a still-live tier-1 child.
        expect(hub.cancelCliLogin).not.toHaveBeenCalled();
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
});
