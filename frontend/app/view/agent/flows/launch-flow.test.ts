// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pins the no-auto-login fix: runLaunchFlow's Phase 2 must NEVER call
 * runProviderLogin (or open a browser/terminal) on its own. Before this
 * fix, an unauthenticated agent's mount-time flow silently launched a
 * login attempt with no click and no way to decline — this is the exact
 * behavior the user reported as broken across multiple test rounds
 * ("it will launch the browser immediately without my clicking login").
 * The flow must instead post a notification, set a terminal phase
 * (first-login / auth-expired), and return "auth_failed" immediately —
 * an actual login only ever starts from the user's own click on the
 * "Log in" button (agent-view.tsx), wired to relogin() in
 * useAgentControllerStatus.ts, not from this function. See
 * docs/specs/SPEC_AGENT_PANE_AUTH_NOTIFICATIONS_2026_07_26.md §8 Q6.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    resolveCli: vi.fn(),
    setMeta: vi.fn(),
    checkCliAuth: vi.fn(),
    listAgentIdentities: vi.fn(),
    getIdentityAccount: vi.fn(),
    controllerResync: vi.fn(),
    getControllerStatus: vi.fn(),
    cancelCliLogin: vi.fn(),
    ensureCapability: vi.fn(),
    getCapability: vi.fn(),
    waveEventSubscribe: vi.fn(),
    getWaveObjectAtom: vi.fn(),
}));

vi.mock("@/app/errors/translate", () => ({
    translateError: (err: any) => ({ title: "Error", message: err?.message ?? String(err), retry: null }),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ResolveCliCommand: (...args: unknown[]) => hub.resolveCli(...args),
        SetMetaCommand: (...args: unknown[]) => hub.setMeta(...args),
        CheckCliAuthCommand: (...args: unknown[]) => hub.checkCliAuth(...args),
        ListAgentIdentitiesCommand: (...args: unknown[]) => hub.listAgentIdentities(...args),
        GetIdentityAccountCommand: (...args: unknown[]) => hub.getIdentityAccount(...args),
        ControllerResyncCommand: (...args: unknown[]) => hub.controllerResync(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/toolchain-capabilities", () => ({
    ensureCapability: (...args: unknown[]) => hub.ensureCapability(...args),
    getCapability: (...args: unknown[]) => hub.getCapability(...args),
}));
vi.mock("@/app/store/wps", () => ({ waveEventSubscribe: (...args: unknown[]) => hub.waveEventSubscribe(...args) }));
vi.mock("@/app/store/wps-events", () => ({ WpsEvent: { InstallProgress: "install_progress" } }));
vi.mock("@/app/store/wos", () => ({
    makeORef: (kind: string, id: string) => `${kind}:${id}`,
    getWaveObjectAtom: (...args: unknown[]) => hub.getWaveObjectAtom(...args),
}));
vi.mock("@/app/store/services", () => ({
    BlockService: { GetControllerStatus: (...args: unknown[]) => hub.getControllerStatus(...args) },
}));
vi.mock("@/app/store/global", () => ({
    getApi: () => ({ cancelCliLogin: hub.cancelCliLogin }),
    staticTabId: () => "tab-1",
}));

import { runLaunchFlow } from "./launch-flow";

const claude = {
    id: "claude",
    displayName: "Claude",
    cliCommand: "claude",
    authCheckCommand: ["auth", "status"],
    authLoginCommand: ["auth", "login"],
    authType: "oauth",
    authConfigDirEnvVar: "CLAUDE_CONFIG_DIR",
    requiresLoginTty: true,
    // No headlessLoginUrlUnsupported: mirrors the real catalog since
    // SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.2 dropped it for Claude
    // (2.1.198+ prints the authorize URL). Inert for these tests either way —
    // launch-flow no longer auto-logins (login starts only from the user's
    // own click), so no tier-1 decision is ever made here.
} as any;

beforeEach(() => {
    hub.resolveCli.mockReset().mockResolvedValue({ cli_path: "x", source: "found", version: "1.0" });
    hub.setMeta.mockReset().mockResolvedValue(undefined);
    hub.checkCliAuth.mockReset().mockResolvedValue({ authenticated: false });
    hub.listAgentIdentities.mockReset().mockResolvedValue([]);
    hub.getIdentityAccount.mockReset().mockResolvedValue({ id: "unused", secret_ref: { backend: "env", env_var: "UNUSED" } });
    hub.controllerResync.mockReset().mockResolvedValue(undefined);
    hub.getControllerStatus.mockReset().mockResolvedValue({ shellprocstatus: "init" });
    hub.cancelCliLogin.mockReset().mockResolvedValue(undefined);
    hub.ensureCapability.mockReset().mockResolvedValue(undefined);
    hub.getCapability.mockReset().mockReturnValue({ status: "available" });
    hub.waveEventSubscribe.mockReset().mockReturnValue(() => {});
    // Default: a brand-new agent that has never resolved its CLI before
    // (no "cmd" in meta) — see the first-login vs auth-expired tests below
    // for the "has run before" case.
    hub.getWaveObjectAtom.mockReset().mockReturnValue(() => ({ meta: { agentMode: "host", agentId: "agent-1" } }));
});
afterEach(() => {
    vi.clearAllMocks();
});

describe("runLaunchFlow — no auto-login on Phase 2", () => {
    it("returns auth_failed immediately on an unauthenticated first-ever login, without opening anything", async () => {
        const phases: string[] = [];
        const notices: Array<{ text: string; style: string }> = [];
        const result = await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            setLaunchPhase: (p) => { if (p) phases.push(p.kind); },
            onNotify: (text, style) => notices.push({ text, style }),
        });

        expect(result).toBe("auth_failed");
        expect(phases).toEqual(["resolving-cli", "checking-auth", "first-login"]);
        // No login attempt of any kind — no terminal-opening / polling phases.
        expect(phases).not.toContain("opening-login-terminal");
        expect(phases).not.toContain("waiting-for-login-link");
        expect(phases).not.toContain("waiting-for-login-completion");
        expect(notices[0].style).toBe("info");
        expect(notices[0].text).toMatch(/sign in/i);
    });

    it("returns auth_failed with auth-expired (not first-login) when a real account link already exists for this agent+provider", async () => {
        hub.listAgentIdentities.mockResolvedValue([{ provider: "claude", account_id: "acct-1" }]);
        const phases: string[] = [];
        const notices: Array<{ text: string; style: string }> = [];
        const result = await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            setLaunchPhase: (p) => { if (p) phases.push(p.kind); },
            onNotify: (text, style) => notices.push({ text, style }),
        });

        expect(result).toBe("auth_failed");
        expect(phases).toContain("auth-expired");
        expect(phases).not.toContain("first-login");
        expect(notices[0].style).toBe("warning");
        expect(notices[0].text).toMatch(/expired/i);
    });

    it("does not throw or hang when onNotify/setLaunchPhase are omitted (both optional)", async () => {
        const result = await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
        });

        expect(result).toBe("auth_failed");
    });
});

describe("runLaunchFlow — Phase 3 (already authenticated)", () => {
    beforeEach(() => {
        hub.checkCliAuth.mockReset().mockResolvedValue({ authenticated: true, email: "user@example.com" });
    });

    it("reports a launchPhase for every visible phase, and never leaves a stale non-terminal phase behind on success", async () => {
        const phases: string[] = [];
        const result = await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            setLaunchPhase: (p) => { if (p) phases.push(p.kind); },
        });

        expect(result).toBe("success");
        expect(phases).toEqual(["resolving-cli", "checking-auth", "verifying", "fresh-ready"]);
    });

    it("sets resumed-ready (not fresh-ready) when GetControllerStatus reports a prior turn (shellprocstatus: done), and posts no transcript notification", async () => {
        // No "Resumed..." notification anymore (removed per
        // docs/specs/REPORT_AGENT_PANE_SYNTHESIZED_TEXT_AUDIT_2026_08_06.md —
        // it narrated nothing the user couldn't already see and rendered
        // indistinguishably from the agent's own words). The phase
        // transition is the behavioral contract that must survive; the
        // notification was never anything more than an artifact of it.
        hub.getControllerStatus.mockResolvedValue({ shellprocstatus: "done" });
        const phases: string[] = [];
        const notices: Array<{ text: string; style: string }> = [];
        const result = await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            setLaunchPhase: (p) => { if (p) phases.push(p.kind); },
            onNotify: (text, style) => notices.push({ text, style }),
        });

        expect(result).toBe("success");
        expect(phases[phases.length - 1]).toBe("resumed-ready");
        expect(phases).not.toContain("fresh-ready");
        expect(notices).toEqual([]);
    });

    it("reagent P1 on PR #2303: also sets resumed-ready for shellprocstatus 'running' — a persistent controller can resume while still alive/mid-turn, not just 'done'", async () => {
        hub.getControllerStatus.mockResolvedValue({ shellprocstatus: "running" });
        const phases: string[] = [];
        const notices: Array<{ text: string; style: string }> = [];
        const result = await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            setLaunchPhase: (p) => { if (p) phases.push(p.kind); },
            onNotify: (text, style) => notices.push({ text, style }),
        });

        expect(result).toBe("success");
        expect(phases[phases.length - 1]).toBe("resumed-ready");
        expect(phases).not.toContain("fresh-ready");
        expect(notices).toEqual([]);
    });

    it("reagent P1 on PR #2304: never posts a cheerful ready/resumed notification when ControllerResync itself fails", async () => {
        // Before this fix, a thrown resync (e.g. the commit-pressure admission
        // gate refusing the controller) was logged but still fell through to
        // the unconditional ready/resumed-ready notification — misrepresenting
        // a failed, possibly-unusable agent as ready.
        hub.controllerResync.mockRejectedValue(new Error("memory full"));
        const notices: Array<{ text: string; style: string }> = [];
        const result = await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            onNotify: (text, style) => notices.push({ text, style }),
        });

        expect(result).toBe("success");
        const last = notices[notices.length - 1];
        expect(last.style).toBe("warning");
        expect(last.text).not.toMatch(/^Ready/i);
        expect(last.text).not.toMatch(/^Resumed/i);
    });

    it("reports onAuthCheckResult(true) when CheckCliAuthCommand actually confirms authentication", async () => {
        const results: boolean[] = [];
        const result = await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            onAuthCheckResult: (confirmed) => results.push(confirmed),
        });

        expect(result).toBe("success");
        expect(results).toEqual([true]);
    });

    it("reagent/codex P2 on PR #2318: reports onAuthCheckResult(false) — not true — when the auth check itself throws, even though the flow still proceeds to 'success'", async () => {
        // Phase 2 deliberately doesn't fail launch on a transient auth-check
        // RPC error ("authentication status unknown — will attempt anyway"),
        // but the caller must be able to tell this apart from an actually
        // confirmed login — otherwise it shows a false "Logged in" tag.
        hub.checkCliAuth.mockReset().mockRejectedValue(new Error("timeout"));
        const results: boolean[] = [];
        const result = await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            onAuthCheckResult: (confirmed) => results.push(confirmed),
        });

        expect(result).toBe("success");
        expect(results).toEqual([false]);
    });
});

describe("runLaunchFlow — mount-time auth check resolves the linked account's own dir", () => {
    // Pins the fix for SPEC_AGENT_PANE_MOUNT_AUTH_CHECK_WRONG_DIR_2026_07_31.md:
    // before this fix, the mount-time check always validated `authEnv`'s
    // generic provider-default dir, even for an agent with a real bound
    // account whose own dir (what the real spawn actually uses) may already
    // be authenticated. Both SetMetaCommand's `cmd:env` and
    // CheckCliAuthCommand's `auth_env` must reflect the linked account's own
    // dir, not the generic one, whenever a link exists. Reads the account's
    // stored `secret_ref.dir` via GetIdentityAccountCommand (codex P1 on PR
    // #2377) rather than reconstructing a path — see that finding's comment
    // in launch-flow.ts for why a reconstructed path can silently diverge
    // from the real stored one.
    it("overrides the generic authEnv dir with the linked account's own stored dir for both SetMetaCommand and CheckCliAuthCommand", async () => {
        hub.listAgentIdentities.mockResolvedValue([{ provider: "claude", account_id: "acct-1" }]);
        hub.getIdentityAccount.mockResolvedValue({
            id: "acct-1",
            secret_ref: { backend: "oauth_config_dir", dir: "/per-account/acct-1" },
        });
        hub.checkCliAuth.mockResolvedValue({ authenticated: true, email: "user@example.com" });

        const result = await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            authEnv: { CLAUDE_CONFIG_DIR: "/generic/shared/dir" },
        });

        expect(result).toBe("success");
        // RpcApi.*Command(client, data, opts?) — the mock hub receives every
        // positional arg, client first.
        expect(hub.getIdentityAccount).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ id: "acct-1" }),
        );
        expect(hub.setMeta).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({
                meta: expect.objectContaining({ "cmd:env": { CLAUDE_CONFIG_DIR: "/per-account/acct-1" } }),
            }),
        );
        expect(hub.checkCliAuth).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ auth_env: { CLAUDE_CONFIG_DIR: "/per-account/acct-1" } }),
            expect.anything(),
        );
        // The linked-account lookup happens once up front and must not be
        // repeated — it was previously only looked up a second time inside
        // the (here, unreached) needsLogin branch.
        expect(hub.listAgentIdentities).toHaveBeenCalledTimes(1);
    });

    it("matches a link row stored under a legacy provider alias (codex P1 on PR #2377)", async () => {
        // A link persisted before providers.rs's alias table existed (or
        // carried forward from an older definition) may still store
        // "claude-code" rather than the canonical "claude" — the lookup must
        // canonicalize before comparing, the same way the backend spawn
        // resolver already does.
        hub.listAgentIdentities.mockResolvedValue([{ provider: "claude-code", account_id: "acct-legacy" }]);
        hub.getIdentityAccount.mockResolvedValue({
            id: "acct-legacy",
            secret_ref: { backend: "oauth_config_dir", dir: "/per-account/acct-legacy" },
        });
        hub.checkCliAuth.mockResolvedValue({ authenticated: true, email: "user@example.com" });

        const result = await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            authEnv: { CLAUDE_CONFIG_DIR: "/generic/shared/dir" },
        });

        expect(result).toBe("success");
        expect(hub.checkCliAuth).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ auth_env: { CLAUDE_CONFIG_DIR: "/per-account/acct-legacy" } }),
            expect.anything(),
        );
    });

    it("prefers the LAST canonical-equivalent link when both a canonical and legacy-alias row exist (codex P1 on PR #2377)", async () => {
        // ListAgentIdentitiesCommand mirrors the backend's own
        // `ORDER BY provider` — canonical "claude" sorts before the alias
        // "claude-code" lexicographically, matching this fixture's order.
        // inject_identity_env's injection loop iterates the same order and
        // overwrites the config-dir env var per binding (plain HashMap
        // insert), so the LAST one — the alias row here — is what the real
        // spawn actually ends up using. Array.prototype.find would wrongly
        // pick the first (canonical) row instead.
        hub.listAgentIdentities.mockResolvedValue([
            { provider: "claude", account_id: "acct-canonical" },
            { provider: "claude-code", account_id: "acct-alias" },
        ]);
        hub.getIdentityAccount.mockImplementation((_client: unknown, data: { id: string }) => {
            if (data.id === "acct-alias") {
                return Promise.resolve({ id: "acct-alias", secret_ref: { backend: "oauth_config_dir", dir: "/per-account/acct-alias" } });
            }
            return Promise.resolve({ id: "acct-canonical", secret_ref: { backend: "oauth_config_dir", dir: "/per-account/acct-canonical" } });
        });
        hub.checkCliAuth.mockResolvedValue({ authenticated: true, email: "user@example.com" });

        const result = await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            authEnv: { CLAUDE_CONFIG_DIR: "/generic/shared/dir" },
        });

        expect(result).toBe("success");
        expect(hub.getIdentityAccount).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({ id: "acct-alias" }));
        expect(hub.checkCliAuth).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ auth_env: { CLAUDE_CONFIG_DIR: "/per-account/acct-alias" } }),
            expect.anything(),
        );
    });

    it("never calls the account-dir lookup for an api-key provider, even one with authConfigDirEnvVar set (codex P2 on PR #2377)", async () => {
        // Kimi is api-key-class but still declares authConfigDirEnvVar for an
        // unrelated purpose — gating on authType (not the env-var field)
        // prevents calling GetIdentityAccountCommand/logging a spurious
        // "no isolated config dir" error for it.
        const kimi = { ...claude, id: "kimi", authType: "api-key", authConfigDirEnvVar: "KIMI_SHARE_DIR" };
        hub.listAgentIdentities.mockResolvedValue([{ provider: "kimi", account_id: "acct-kimi" }]);
        hub.checkCliAuth.mockResolvedValue({ authenticated: true, email: "user@example.com" });

        const result = await runLaunchFlow({
            blockId: "block-1",
            provider: kimi,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            authEnv: { KIMI_SHARE_DIR: "/generic/shared/dir" },
        });

        expect(result).toBe("success");
        expect(hub.listAgentIdentities).not.toHaveBeenCalled();
        expect(hub.getIdentityAccount).not.toHaveBeenCalled();
        expect(hub.checkCliAuth).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ auth_env: { KIMI_SHARE_DIR: "/generic/shared/dir" } }),
            expect.anything(),
        );
    });

    it("falls back to the generic authEnv dir when no account is linked yet (genuine first-login)", async () => {
        hub.listAgentIdentities.mockResolvedValue([]);
        hub.checkCliAuth.mockResolvedValue({ authenticated: true, email: "user@example.com" });

        const result = await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            authEnv: { CLAUDE_CONFIG_DIR: "/generic/shared/dir" },
        });

        expect(result).toBe("success");
        expect(hub.getIdentityAccount).not.toHaveBeenCalled();
        expect(hub.setMeta).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({
                meta: expect.objectContaining({ "cmd:env": { CLAUDE_CONFIG_DIR: "/generic/shared/dir" } }),
            }),
        );
        expect(hub.checkCliAuth).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ auth_env: { CLAUDE_CONFIG_DIR: "/generic/shared/dir" } }),
            expect.anything(),
        );
    });

    it("falls back to the generic authEnv dir when the linked account's secret_ref isn't an oauth config dir (soft failure)", async () => {
        hub.listAgentIdentities.mockResolvedValue([{ provider: "claude", account_id: "acct-1" }]);
        hub.getIdentityAccount.mockResolvedValue({ id: "acct-1", secret_ref: { backend: "env", env_var: "SOMETHING" } });
        hub.checkCliAuth.mockResolvedValue({ authenticated: true, email: "user@example.com" });

        const result = await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            authEnv: { CLAUDE_CONFIG_DIR: "/generic/shared/dir" },
        });

        expect(result).toBe("success");
        expect(hub.checkCliAuth).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ auth_env: { CLAUDE_CONFIG_DIR: "/generic/shared/dir" } }),
            expect.anything(),
        );
    });

    it("falls back to the generic authEnv dir when GetIdentityAccountCommand itself throws (soft failure)", async () => {
        hub.listAgentIdentities.mockResolvedValue([{ provider: "claude", account_id: "acct-1" }]);
        hub.getIdentityAccount.mockRejectedValue(new Error("not found"));
        hub.checkCliAuth.mockResolvedValue({ authenticated: true, email: "user@example.com" });

        const result = await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            authEnv: { CLAUDE_CONFIG_DIR: "/generic/shared/dir" },
        });

        expect(result).toBe("success");
        expect(hub.checkCliAuth).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ auth_env: { CLAUDE_CONFIG_DIR: "/generic/shared/dir" } }),
            expect.anything(),
        );
    });
});
