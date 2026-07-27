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
    authConfigDirEnvVar: "CLAUDE_CONFIG_DIR",
    requiresLoginTty: true,
    headlessLoginUrlUnsupported: true,
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

    it("notifies 'Resumed...' (not 'Ready...') when GetControllerStatus reports a prior turn (shellprocstatus: done)", async () => {
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
});
