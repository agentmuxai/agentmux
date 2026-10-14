// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    checkCliAuth: vi.fn(),
    resolveCli: vi.fn(),
    listAgentIdentities: vi.fn(),
    runProviderLogin: vi.fn(),
    persistAndLinkAccount: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        CheckCliAuthCommand: (...args: unknown[]) => hub.checkCliAuth(...args),
        ResolveCliCommand: (...args: unknown[]) => hub.resolveCli(...args),
        ListAgentIdentitiesCommand: (...args: unknown[]) => hub.listAgentIdentities(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("./run-provider-login", () => ({
    runProviderLogin: (...args: unknown[]) => hub.runProviderLogin(...args),
    persistAndLinkAccount: (...args: unknown[]) => hub.persistAndLinkAccount(...args),
}));

import {
    LOGIN_COMPLETION_TIMEOUT_MS,
    linkedAccountFor,
    logInAndConfirm,
    pollUntilAuthenticated,
    resolveProviderCli,
} from "./login-and-confirm";

const provider = {
    id: "claude",
    cliCommand: "claude",
    authCheckCommand: ["auth", "status"],
    authConfigDirEnvVar: "CLAUDE_CONFIG_DIR",
} as any;

function params(over: Record<string, unknown> = {}) {
    return {
        provider,
        cliPath: "/bin/claude",
        authEnv: { CLAUDE_CONFIG_DIR: "/generic" },
        setAuthUrl: vi.fn(),
        log: vi.fn(),
        isCancelled: () => false,
        linkTarget: { blockId: "b1", agentDefinitionId: "agent-1" },
        ...over,
    } as any;
}

/** runProviderLogin that registers an account (or not) and returns `outcome`. */
function providerLogin(outcome: string, account?: [string, string]) {
    hub.runProviderLogin.mockImplementation(async (p: any) => {
        if (account) p.onAccountRegistered?.(...account);
        return outcome;
    });
}

beforeEach(() => {
    hub.checkCliAuth.mockReset().mockResolvedValue({ authenticated: true, email: "me@example.com" });
    hub.persistAndLinkAccount.mockReset().mockResolvedValue(true);
    hub.runProviderLogin.mockReset();
});
afterEach(() => {
    vi.useRealTimers();
});

describe("logInAndConfirm — outcomes that settle inside runProviderLogin", () => {
    it.each([
        ["inapp-success", ["acct", "/dir"], { kind: "signed-in", email: null }],
        ["terminal-success", ["acct", "/dir"], { kind: "signed-in", email: null }],
        // The account row couldn't be saved: never report success for it.
        ["inapp-success", undefined, { kind: "save-failed" }],
        ["terminal-success", undefined, { kind: "save-failed" }],
        // A CLI that keeps its own login has no account to save.
        ["terminal-cli-signed-in", undefined, { kind: "signed-in", email: null }],
        ["inapp-timeout", undefined, { kind: "not-detected", stage: "inapp" }],
        ["terminal-timeout", undefined, { kind: "not-detected", stage: "terminal" }],
        ["terminal-unavailable", undefined, { kind: "unavailable" }],
    ] as const)("%s (account %j) → %j", async (outcome, account, expected) => {
        providerLogin(outcome, account as [string, string] | undefined);
        expect(await logInAndConfirm(params())).toEqual(expected);
        expect(hub.persistAndLinkAccount).not.toHaveBeenCalled();
    });

    it("passes the caller's options through and still reports the account to the caller", async () => {
        const onAccountRegistered = vi.fn();
        const onTierChange = vi.fn();
        providerLogin("terminal-success", ["acct", "/dir"]);
        await logInAndConfirm(params({ skipTier1: true, existingAccountId: "old", onAccountRegistered, onTierChange }));
        expect(hub.runProviderLogin).toHaveBeenCalledWith(
            expect.objectContaining({ skipTier1: true, existingAccountId: "old", cliPath: "/bin/claude", onTierChange }),
        );
        expect(onAccountRegistered).toHaveBeenCalledWith("acct", "/dir");
    });
});

describe("logInAndConfirm — tier 1 'opened': poll, then persist", () => {
    it("polls the account's own dir, persists it, and reports the signed-in email", async () => {
        vi.useFakeTimers();
        providerLogin("opened", ["acct", "/acct-dir"]);
        const onTierChange = vi.fn();
        const p = params({ onTierChange });
        const result = logInAndConfirm(p);
        await vi.advanceTimersByTimeAsync(2_000);
        expect(await result).toEqual({ kind: "signed-in", email: "me@example.com" });
        expect(hub.checkCliAuth).toHaveBeenCalledWith(
            {},
            expect.objectContaining({ auth_env: { CLAUDE_CONFIG_DIR: "/acct-dir" } }),
            { timeout: 10000 },
        );
        expect(hub.persistAndLinkAccount).toHaveBeenCalledWith(
            expect.objectContaining({ authEnv: { CLAUDE_CONFIG_DIR: "/generic" }, linkTarget: p.linkTarget }),
            "acct",
            "/acct-dir",
        );
        expect(onTierChange).toHaveBeenCalledWith({ tier: "polling", deadlineMs: expect.any(Number) });
        expect(p.log).toHaveBeenCalledWith("auth", "waiting for login to complete...");
    });

    it("save-failed when the account can't be persisted", async () => {
        vi.useFakeTimers();
        providerLogin("opened", ["acct", "/acct-dir"]);
        hub.persistAndLinkAccount.mockResolvedValue(false);
        const result = logInAndConfirm(params());
        await vi.advanceTimersByTimeAsync(2_000);
        expect(await result).toEqual({ kind: "save-failed" });
    });

    it("not-detected after the 5-minute window", async () => {
        vi.useFakeTimers();
        providerLogin("opened", ["acct", "/acct-dir"]);
        hub.checkCliAuth.mockResolvedValue({ authenticated: false });
        const result = logInAndConfirm(params());
        await vi.advanceTimersByTimeAsync(LOGIN_COMPLETION_TIMEOUT_MS);
        expect(await result).toEqual({ kind: "not-detected", stage: "link" });
        expect(hub.persistAndLinkAccount).not.toHaveBeenCalled();
    });

    it("signed in but no minted account (its dir mint failed) is not-detected, never success", async () => {
        vi.useFakeTimers();
        providerLogin("opened");
        const result = logInAndConfirm(params());
        await vi.advanceTimersByTimeAsync(2_000);
        expect(await result).toEqual({ kind: "not-detected", stage: "link" });
    });

    it("stops at once on cancel instead of running out the window", async () => {
        vi.useFakeTimers();
        providerLogin("opened", ["acct", "/acct-dir"]);
        hub.checkCliAuth.mockResolvedValue({ authenticated: false });
        let cancelled = false;
        const result = logInAndConfirm(params({ isCancelled: () => cancelled }));
        await vi.advanceTimersByTimeAsync(4_000);
        cancelled = true;
        await vi.advanceTimersByTimeAsync(2_000);
        expect(await result).toEqual({ kind: "not-detected", stage: "link" });
        expect(hub.checkCliAuth).toHaveBeenCalledTimes(2);
    });
});

describe("pollUntilAuthenticated", () => {
    it("keeps polling through transient RPC errors", async () => {
        vi.useFakeTimers();
        hub.checkCliAuth
            .mockRejectedValueOnce(new Error("timeout"))
            .mockResolvedValueOnce({ authenticated: false })
            .mockResolvedValueOnce({ authenticated: true });
        const result = pollUntilAuthenticated({
            cliPath: "c",
            authCheckArgs: [],
            authEnv: {},
            isCancelled: () => false,
            deadlineMs: Date.now() + 60_000,
        });
        await vi.advanceTimersByTimeAsync(6_000);
        expect(await result).toEqual({ email: null });
    });

    it("returns null without checking when already cancelled", async () => {
        const result = await pollUntilAuthenticated({
            cliPath: "c",
            authCheckArgs: [],
            authEnv: {},
            isCancelled: () => true,
            deadlineMs: Date.now() + 60_000,
        });
        expect(result).toBeNull();
        expect(hub.checkCliAuth).not.toHaveBeenCalled();
    });
});

describe("shared provider steps", () => {
    it("resolveProviderCli sends the provider's install recipe and a 5-minute timeout", async () => {
        hub.resolveCli.mockResolvedValue({ cli_path: "/x" });
        await resolveProviderCli(
            { id: "codex", cliCommand: "codex", npmPackage: "@openai/codex", pinnedVersion: "1.0" } as any,
            "block-1",
        );
        expect(hub.resolveCli).toHaveBeenCalledWith(
            {},
            expect.objectContaining({ provider_id: "codex", cli_command: "codex", npm_package: "@openai/codex", pinned_version: "1.0", block_id: "block-1" }),
            { timeout: 300000 },
        );
    });

    it("linkedAccountFor matches a legacy alias, and is undefined when the lookup fails", async () => {
        hub.listAgentIdentities.mockResolvedValueOnce([{ provider: "claude-code", account_id: "acct-alias" }]);
        expect(await linkedAccountFor("agent-1", "claude")).toBe("acct-alias");
        hub.listAgentIdentities.mockRejectedValueOnce(new Error("down"));
        expect(await linkedAccountFor("agent-1", "claude")).toBeUndefined();
    });
});
