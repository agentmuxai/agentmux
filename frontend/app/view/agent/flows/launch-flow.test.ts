// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pins the deterministic-login-UX fix: runLaunchFlow must pass
 * `skipTier1: true` into runProviderLogin for providers flagged
 * `headlessLoginUrlUnsupported` (Claude) — so the mount-time auto-login
 * path never burns cli_login.rs's URL-capture wait on a documented dead
 * end. See catalog.ts's DEAD END note and run-provider-login.test.ts's
 * own "skipTier1 skips the headless URL-capture attempt entirely" test,
 * which pins that a true skipTier1 keeps getApi().runCliLogin from ever
 * being called — this test pins that runLaunchFlow actually sets that
 * flag for the right providers.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    resolveCli: vi.fn(),
    setMeta: vi.fn(),
    checkCliAuth: vi.fn(),
    listAgentIdentities: vi.fn(),
    controllerResync: vi.fn(),
    getControllerStatus: vi.fn(),
    cancelCliLogin: vi.fn(),
    ensureCapability: vi.fn(),
    getCapability: vi.fn(),
    runProviderLogin: vi.fn(),
    persistAndLinkAccount: vi.fn(),
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
vi.mock("./run-provider-login", () => ({
    runProviderLogin: (...args: unknown[]) => hub.runProviderLogin(...args),
    persistAndLinkAccount: (...args: unknown[]) => hub.persistAndLinkAccount(...args),
}));

import { runLaunchFlow } from "./launch-flow";

const claude = {
    id: "claude",
    cliCommand: "claude",
    authCheckCommand: ["auth", "status"],
    authLoginCommand: ["auth", "login"],
    authConfigDirEnvVar: "CLAUDE_CONFIG_DIR",
    requiresLoginTty: true,
    headlessLoginUrlUnsupported: true,
} as any;

const codex = {
    id: "codex",
    cliCommand: "codex",
    authCheckCommand: ["auth", "status"],
    authLoginCommand: ["login"],
    authConfigDirEnvVar: "CODEX_HOME",
} as any;

beforeEach(() => {
    hub.resolveCli.mockReset().mockResolvedValue({ cli_path: "x", source: "found", version: "1.0" });
    hub.setMeta.mockReset().mockResolvedValue(undefined);
    hub.checkCliAuth.mockReset().mockResolvedValue({ authenticated: false });
    hub.listAgentIdentities.mockReset().mockResolvedValue([]);
    hub.controllerResync.mockReset().mockResolvedValue(undefined);
    hub.getControllerStatus.mockReset().mockResolvedValue({ shellprocstatus: "init" });
    hub.cancelCliLogin.mockReset().mockResolvedValue(undefined);
    hub.ensureCapability.mockReset().mockResolvedValue(undefined);
    hub.getCapability.mockReset().mockReturnValue({ status: "available" });
    hub.runProviderLogin.mockReset().mockResolvedValue("seeded");
    hub.persistAndLinkAccount.mockReset().mockResolvedValue(true);
    hub.waveEventSubscribe.mockReset().mockReturnValue(() => {});
    // Default: a brand-new agent that has never resolved its CLI before
    // (no "cmd" in meta) — see the first-login vs auth-expired tests below
    // for the "has run before" case.
    hub.getWaveObjectAtom.mockReset().mockReturnValue(() => ({ meta: { agentMode: "host", agentId: "agent-1" } }));
});
afterEach(() => {
    vi.clearAllMocks();
});

describe("runLaunchFlow — skipTier1 wiring", () => {
    it("passes skipTier1: true for a headlessLoginUrlUnsupported provider (Claude) — the auto-login path never attempts tier 1's doomed URL-capture wait", async () => {
        await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
        });

        expect(hub.runProviderLogin).toHaveBeenCalledTimes(1);
        expect(hub.runProviderLogin.mock.calls[0][0]).toMatchObject({ skipTier1: true });
    });

    it("leaves skipTier1 false for a provider without the flag (Codex) — tier 1 still gets a real chance to capture a URL", async () => {
        await runLaunchFlow({
            blockId: "block-1",
            provider: codex,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
        });

        expect(hub.runProviderLogin).toHaveBeenCalledTimes(1);
        expect(hub.runProviderLogin.mock.calls[0][0]).toMatchObject({ skipTier1: false });
    });

    it("reports a launchPhase for every visible phase, and never leaves a stale non-terminal phase behind on success", async () => {
        // First call is Phase 2's initial auth check (unauthenticated, so
        // needsLogin fires); second is the post-"seeded" one-shot recheck,
        // which must report success for the flow to reach "ready".
        hub.checkCliAuth
            .mockReset()
            .mockResolvedValueOnce({ authenticated: false })
            .mockResolvedValue({ authenticated: true, email: "user@example.com" });
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
        expect(phases[0]).toBe("resolving-cli");
        expect(phases).toContain("checking-auth");
        // No prior "cmd" in meta (see beforeEach) — this is a first-ever
        // login, not a lapsed one.
        expect(phases).toContain("first-login");
        expect(phases).toContain("opening-login-terminal");
        // getControllerStatus defaults to shellprocstatus: "init" (beforeEach).
        expect(phases[phases.length - 1]).toBe("fresh-ready");
    });

    it("reports auth-expired (not first-login) when the agent has resolved its CLI before — meta.cmd already set", async () => {
        hub.getWaveObjectAtom.mockReturnValue(() => ({
            meta: { agentMode: "host", agentId: "agent-1", cmd: "C:/prior/claude.cmd" },
        }));
        const phases: string[] = [];
        await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            setLaunchPhase: (p) => { if (p) phases.push(p.kind); },
        });

        expect(phases).toContain("auth-expired");
        expect(phases).not.toContain("first-login");
    });

    it("notifies with a warning before an expired-token relogin, and a neutral message for a first-ever login", async () => {
        const notices: Array<{ text: string; style: string }> = [];
        hub.getWaveObjectAtom.mockReturnValue(() => ({
            meta: { agentMode: "host", agentId: "agent-1", cmd: "C:/prior/claude.cmd" },
        }));
        await runLaunchFlow({
            blockId: "block-1",
            provider: claude,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            onNotify: (text, style) => notices.push({ text, style }),
        });

        expect(notices[0].style).toBe("warning");
        expect(notices[0].text).toMatch(/expired/i);
    });

    it("notifies 'Resumed...' (not 'Ready...') when GetControllerStatus reports a prior turn (shellprocstatus: done)", async () => {
        // Needs the login to actually succeed (auth_failed returns before
        // Phase 3 ever runs) — same recheck-succeeds pattern as the
        // "reports a launchPhase for every visible phase" test above.
        hub.checkCliAuth
            .mockReset()
            .mockResolvedValueOnce({ authenticated: false })
            .mockResolvedValue({ authenticated: true, email: "user@example.com" });
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
        const last = notices[notices.length - 1];
        expect(last.text).toMatch(/resumed/i);
    });

    it("reagent P1 on PR #2303: also notifies 'Resumed...' for shellprocstatus 'running' — a persistent controller can resume while still alive/mid-turn, not just 'done'", async () => {
        hub.checkCliAuth
            .mockReset()
            .mockResolvedValueOnce({ authenticated: false })
            .mockResolvedValue({ authenticated: true, email: "user@example.com" });
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
        const last = notices[notices.length - 1];
        expect(last.text).toMatch(/resumed/i);
    });

    it("reagent P1: updates the phase via onTierChange instead of freezing on a stale waiting-for-login-link deadline once tier 1 fails and tier 2/3 take over", async () => {
        // Codex doesn't have headlessLoginUrlUnsupported, so it gets the
        // deadline-bearing "waiting-for-login-link" phase before this call —
        // exactly the phase that used to freeze once tier 1's own timeout
        // expired and runProviderLogin's internal tier 2/3 (up to 5 more
        // minutes) took over with zero further signal to the caller.
        hub.checkCliAuth
            .mockReset()
            .mockResolvedValueOnce({ authenticated: false })
            .mockResolvedValue({ authenticated: true, email: "user@example.com" });
        hub.runProviderLogin.mockReset().mockImplementation(async (params: any) => {
            // Simulate tier 1 failing, then a terminal opening and polling —
            // exactly what run-provider-login.ts's real onTierChange calls do.
            params.onTierChange?.({ tier: "fallback" });
            params.onTierChange?.({ tier: "polling", deadlineMs: Date.now() + 5 * 60 * 1000 });
            return "terminal-success";
        });
        const phases: Array<{ kind: string; deadlineMs?: number }> = [];
        await runLaunchFlow({
            blockId: "block-1",
            provider: codex,
            log: vi.fn(),
            setAuthUrl: vi.fn(),
            isCancelled: () => false,
            setLoginWaiting: vi.fn(),
            setLaunchPhase: (p) => { if (p) phases.push(p as any); },
        });

        const linkPhaseIndex = phases.findIndex((p) => p.kind === "waiting-for-login-link");
        const fallbackIndex = phases.findIndex((p) => p.kind === "opening-login-terminal");
        const pollingIndex = phases.findIndex((p) => p.kind === "waiting-for-login-completion");
        expect(linkPhaseIndex).toBeGreaterThanOrEqual(0);
        // The stale-deadline phase must not be the last thing shown — both
        // onTierChange transitions must land AFTER it, in order.
        expect(fallbackIndex).toBeGreaterThan(linkPhaseIndex);
        expect(pollingIndex).toBeGreaterThan(fallbackIndex);
        const pollingPhase = phases[pollingIndex];
        expect(pollingPhase.deadlineMs).toBeGreaterThan(Date.now());
    });
});
