// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentControllerStatus — characterisation of what the hook does around the launch-phase
 * reducer: the arguments it hands the launch flow and the provider login, the order of its steps,
 * what it logs and where, the auth env it builds, the bind, the recheck and the terminal escape.
 * Written ahead of a split of the hook. The provider flows are mocked so each argument can be
 * read; the launch-phase store is the real one, so the state the hook reports is the state it gets.
 */

import { __resetDispatchLog, getRecentDispatches } from "@/app/store/command-source";
import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    meta: {} as Record<string, unknown>,
    provider: undefined as unknown,
    order: [] as string[],
    sleeps: [] as number[],
    windowProvider: undefined as unknown,
    windowCalls: 0,
    runLaunchFlow: vi.fn(),
    prestartContainerAgent: vi.fn(),
    resolveProviderCli: vi.fn(),
    checkCliAuth: vi.fn(),
    linkedAccountFor: vi.fn(),
    logInAndConfirm: vi.fn(),
    registerController: vi.fn(),
    ensureProviderAuthDir: vi.fn(),
    cancel: vi.fn(),
    setMeta: vi.fn(),
    accountSwitch: vi.fn(),
    notifyLinksChanged: vi.fn(),
}));

vi.mock("@/app/store/block-layout-actions", () => ({}));
vi.mock("@/app/store/global", () => ({
    getApi: () => ({}),
    getBlockMetaKeyAtom: (blockId: string, key: string) => () => hub.meta[`${blockId}|${key}`],
    staticTabId: () => "tab-1",
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SetMetaCommand: (...a: unknown[]) => hub.setMeta(...a),
        AgentAccountSwitchCommand: (...a: unknown[]) => hub.accountSwitch(...a),
    },
}));
vi.mock("@/app/store/mos", () => ({ makeORef: (a: string, b: string) => `${a}:${b}` }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: { tag: "rpc-client" } }));
vi.mock("@/app/store/services", () => ({ BlockService: {} }));
vi.mock("@/app/view/identity/account-agents", () => ({
    notifyLinksChanged: (...a: unknown[]) => hub.notifyLinksChanged(...a),
}));
vi.mock("@/util/util", async (importOriginal) => {
    const real = await importOriginal<typeof import("@/util/util")>();
    return {
        ...real,
        sleep: (ms: number) => {
            hub.sleeps.push(ms);
            return new Promise<void>((r) => setTimeout(r, ms));
        },
    };
});
vi.mock("../agent-launch-env", () => ({ ensureProviderAuthDir: (...a: unknown[]) => hub.ensureProviderAuthDir(...a) }));
vi.mock("../flows/launch-flow", () => ({
    runLaunchFlow: (...a: unknown[]) => hub.runLaunchFlow(...a),
    prestartContainerAgent: (...a: unknown[]) => hub.prestartContainerAgent(...a),
}));
vi.mock("../flows/login-and-confirm", () => ({
    resolveProviderCli: (...a: unknown[]) => hub.resolveProviderCli(...a),
    checkCliAuth: (...a: unknown[]) => hub.checkCliAuth(...a),
    linkedAccountFor: (...a: unknown[]) => hub.linkedAccountFor(...a),
    logInAndConfirm: (...a: unknown[]) => hub.logInAndConfirm(...a),
}));
vi.mock("../flows/register-controller", () => ({
    registerController: (...a: unknown[]) => hub.registerController(...a),
}));
vi.mock("../flows/login-backend", () => ({ loginBackend: () => ({ cancel: (...a: unknown[]) => hub.cancel(...a) }) }));
vi.mock("../flows/login-window", () => ({
    withLoginWindow: (prov: unknown, run: () => Promise<void>) => {
        hub.windowProvider = prov;
        hub.windowCalls++;
        return run();
    },
}));

import {
    useAgentControllerStatus,
    type UseAgentControllerStatus,
    type UseAgentControllerStatusOptions,
} from "./useAgentControllerStatus";

const BLOCK = "blk-1";
type Prov = Record<string, unknown>;
const claude = (over: Prov = {}): Prov => ({
    id: "claude",
    cliCommand: "claude",
    authCheckCommand: "claude auth status",
    authConfigDirEnvVar: "CLAUDE_CONFIG_DIR",
    authDirName: ".claude",
    authExtraEnv: { EXTRA: "1" },
    ...over,
});
const setMeta = (key: string, value: unknown) => {
    hub.meta[`${BLOCK}|${key}`] = value;
};
const flush = async () => {
    for (let i = 0; i < 12; i++) await Promise.resolve();
};
const deferred = <T = void,>() => {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return { promise, resolve, reject };
};

async function inRoot(fn: () => Promise<void>) {
    await createRoot(async (dispose) => {
        try {
            await fn();
        } finally {
            dispose();
        }
    });
}

function mk(over: Partial<UseAgentControllerStatusOptions> = {}) {
    const cb = {
        log: vi.fn(),
        onLoginSuccess: vi.fn(() => {
            hub.order.push("onLoginSuccess");
        }),
        onNotify: vi.fn(),
        onAccountBound: vi.fn(),
        onReady: vi.fn(),
        onRecovered: vi.fn(),
        getInitialTermSize: vi.fn(() => ({ rows: 24, cols: 80 })),
        onControllerStatus: vi.fn(),
    };
    const status = useAgentControllerStatus({ blockId: BLOCK, provider: () => hub.provider as never, ...cb, ...over });
    return { status, ...cb };
}

const loginArgs = () =>
    hub.logInAndConfirm.mock.calls[0][0] as Record<string, unknown> & {
        setAuthUrl: (u: string | null) => void;
        isCancelled: () => boolean;
        onTierChange: (e: { tier: string; deadlineMs?: number }) => void;
    };

beforeEach(() => {
    __resetDispatchLog();
    for (const k of Object.keys(hub.meta)) delete hub.meta[k];
    hub.provider = claude();
    hub.order.length = 0;
    hub.sleeps.length = 0;
    hub.windowProvider = undefined;
    hub.windowCalls = 0;
    for (const f of [
        hub.runLaunchFlow,
        hub.prestartContainerAgent,
        hub.resolveProviderCli,
        hub.checkCliAuth,
        hub.linkedAccountFor,
        hub.logInAndConfirm,
        hub.registerController,
        hub.ensureProviderAuthDir,
        hub.cancel,
        hub.setMeta,
        hub.accountSwitch,
        hub.notifyLinksChanged,
    ]) {
        f.mockReset();
    }
    hub.runLaunchFlow.mockResolvedValue("success");
    hub.resolveProviderCli.mockResolvedValue({ cli_path: "/resolved/claude" });
    hub.checkCliAuth.mockResolvedValue({ authenticated: true });
    hub.linkedAccountFor.mockResolvedValue(undefined);
    hub.logInAndConfirm.mockResolvedValue({ kind: "signed-in", email: "me@x.com" });
    hub.registerController.mockImplementation(async () => {
        hub.order.push("registerController");
        return { status: "rts" };
    });
    hub.ensureProviderAuthDir.mockResolvedValue("/auth/claude");
    hub.cancel.mockResolvedValue(undefined);
    hub.setMeta.mockResolvedValue(undefined);
    hub.accountSwitch.mockResolvedValue({ failed: [], status: "done", account_label: "Work" });
    setMeta("cmd", "/bin/claude");
});
afterEach(() => {
    vi.useRealTimers();
});

describe("the launch flow it starts", () => {
    it("hands the flow this pane's block, provider, handlers and the provider's auth env", async () => {
        await inRoot(async () => {
            const { status, log, onNotify, getInitialTermSize, onControllerStatus } = mk();
            await status.startLaunchFlow();
            expect(hub.runLaunchFlow).toHaveBeenCalledTimes(1);
            const a = hub.runLaunchFlow.mock.calls[0][0];
            expect(a.blockId).toBe(BLOCK);
            expect(a.provider).toBe(hub.provider);
            expect(a.log).toBe(log);
            expect(a.onNotify).toBe(onNotify);
            expect(a.getInitialTermSize).toBe(getInitialTermSize);
            expect(a.onControllerStatus).toBe(onControllerStatus);
            expect(await a.authEnv).toEqual({ CLAUDE_CONFIG_DIR: "/auth/claude", EXTRA: "1" });
            expect(hub.ensureProviderAuthDir).toHaveBeenCalledWith("claude");
        });
    });

    it.each([
        ["no env var name", { authConfigDirEnvVar: undefined }],
        ["no auth dir name", { authDirName: undefined }],
    ])("builds no auth env for a provider with %s", async (_n, over) => {
        hub.provider = claude(over);
        await inRoot(async () => {
            const { status } = mk();
            await status.startLaunchFlow();
            expect(await hub.runLaunchFlow.mock.calls[0][0].authEnv).toBeUndefined();
            expect(hub.ensureProviderAuthDir).not.toHaveBeenCalled();
        });
    });

    it("builds the env without extras when the provider has none, and none when the dir can't be made", async () => {
        hub.provider = claude({ authExtraEnv: undefined });
        await inRoot(async () => {
            const { status } = mk();
            await status.startLaunchFlow();
            expect(await hub.runLaunchFlow.mock.calls[0][0].authEnv).toEqual({ CLAUDE_CONFIG_DIR: "/auth/claude" });
        });
        hub.provider = claude();
        hub.ensureProviderAuthDir.mockRejectedValue(new Error("no disk"));
        await inRoot(async () => {
            const { status } = mk();
            await status.startLaunchFlow();
            expect(await hub.runLaunchFlow.mock.calls.at(-1)![0].authEnv).toBeUndefined();
        });
    });

    it("shows the phase the flow reports, and ends clean after a success", async () => {
        let during: unknown;
        let loadingDuring: boolean | undefined;
        await inRoot(async () => {
            const h = mk();
            hub.runLaunchFlow.mockImplementation(async (o: { setLaunchPhase: (p: unknown) => void }) => {
                o.setLaunchPhase({ kind: "verifying" });
                during = h.status.launchPhase();
                loadingDuring = h.status.isLoading();
                return "success";
            });
            await h.status.startLaunchFlow();
            expect(during).toEqual({ kind: "verifying" });
            expect(loadingDuring).toBe(true);
            expect(h.onReady).toHaveBeenCalledTimes(1);
            expect(h.status.flowRunning()).toBe(false);
            expect(h.status.launchPhase()).toBeNull();
        });
    });

    it("tags the pane signed in only when the flow confirmed the check", async () => {
        await inRoot(async () => {
            const { status } = mk();
            await status.startLaunchFlow();
            expect(status.authStatus()).toBe("unknown");
        });
        hub.runLaunchFlow.mockImplementation(async (o: { onAuthCheckResult: (c: boolean) => void }) => {
            o.onAuthCheckResult(true);
            return "success";
        });
        await inRoot(async () => {
            const { status } = mk();
            await status.startLaunchFlow();
            expect(status.authStatus()).toBe("authenticated");
        });
        hub.runLaunchFlow.mockImplementation(async (o: { onAuthCheckResult: (c: boolean) => void }) => {
            o.onAuthCheckResult(false);
            return "success";
        });
        await inRoot(async () => {
            const { status } = mk();
            await status.startLaunchFlow();
            expect(status.authStatus()).toBe("unknown");
        });
    });

    it.each([["auth_failed"], ["fatal"]])("does not call onReady after a %s launch", async (result) => {
        hub.runLaunchFlow.mockResolvedValue(result);
        await inRoot(async () => {
            const { status, onReady } = mk();
            await status.startLaunchFlow();
            expect(onReady).not.toHaveBeenCalled();
            expect(status.flowRunning()).toBe(false);
            expect(status.canRetry()).toBe(result === "auth_failed");
        });
    });

    it("logs a launch that threw as an error, with the message or the text, and does not mark it signed out", async () => {
        for (const [thrown, text] of [
            [new Error("boom"), "boom"],
            ["plain words", "plain words"],
        ] as const) {
            hub.runLaunchFlow.mockRejectedValue(thrown);
            await inRoot(async () => {
                const { status, log, onReady } = mk();
                await status.startLaunchFlow();
                expect(log).toHaveBeenCalledWith("error", text, "error");
                expect(onReady).not.toHaveBeenCalled();
                expect(status.canRetry()).toBe(false);
                expect(status.flowRunning()).toBe(false);
            });
        }
    });

    it("logs an onReady that threw, and still ends the flow", async () => {
        await inRoot(async () => {
            const { status, log } = mk({
                onReady: () => {
                    throw new Error("ready blew up");
                },
            });
            await status.startLaunchFlow();
            expect(log).toHaveBeenCalledWith("error", "ready blew up", "error");
            expect(status.flowRunning()).toBe(false);
        });
    });
});

describe("the pane's own login", () => {
    it("hands the provider login the CLI, the env, the account link and the pane's callbacks", async () => {
        setMeta("agentId", "def-1");
        setMeta("cmd:env", { CLAUDE_CONFIG_DIR: "/acct/dir", NOT_A_STRING: 5, NULLISH: null });
        hub.linkedAccountFor.mockResolvedValue("acct-9");
        await inRoot(async () => {
            const { status, log } = mk();
            await status.relogin({ retryAfterLogin: false });
            const a = loginArgs();
            expect(a.provider).toBe(hub.provider);
            expect(a.cliPath).toBe("/bin/claude");
            expect(await a.authEnv).toEqual({ CLAUDE_CONFIG_DIR: "/acct/dir" });
            expect(a.linkTarget).toEqual({ blockId: BLOCK, agentDefinitionId: "def-1" });
            expect(a.existingAccountId).toBe("acct-9");
            expect(hub.linkedAccountFor).toHaveBeenCalledWith("def-1", "claude");
            expect(a.log).toBe(log);
            expect(a.skipTier1).toBe(false);
            expect(a.awaitTier1Completion).toBe(true);
            a.setAuthUrl("https://login.example/x");
            expect(status.authUrl()).toBe("https://login.example/x");
        });
    });

    it("links nothing and looks up no account when the pane has no agent definition", async () => {
        await inRoot(async () => {
            const { status } = mk();
            await status.relogin();
            const a = loginArgs();
            expect(a.linkTarget).toBeUndefined();
            expect(a.existingAccountId).toBeUndefined();
            expect(hub.linkedAccountFor).not.toHaveBeenCalled();
        });
    });

    it("asks for the awaited in-app session on a relogin only, and skips link capture for a terminal login", async () => {
        await inRoot(async () => {
            const { status } = mk();
            await status.loginViaTerminal({ retryAfterLogin: false });
            const a = loginArgs();
            expect(a.skipTier1).toBe(true);
            expect(a).not.toHaveProperty("awaitTier1Completion");
        });
    });

    it("skips link capture for a provider that cannot print a link, even on a relogin", async () => {
        hub.provider = claude({ headlessLoginUrlUnsupported: true });
        await inRoot(async () => {
            const { status } = mk();
            await status.relogin();
            expect(loginArgs().skipTier1).toBe(true);
            expect(loginArgs().awaitTier1Completion).toBe(true);
        });
        hub.logInAndConfirm.mockClear();
        hub.provider = claude({ headlessLoginUrlUnsupported: undefined });
        await inRoot(async () => {
            const { status } = mk();
            await status.relogin();
            expect(loginArgs().skipTier1).toBe(false);
        });
    });

    it("opens the login window for the pane's provider", async () => {
        await inRoot(async () => {
            const { status } = mk();
            await status.relogin();
            expect(hub.windowCalls).toBe(1);
            expect(hub.windowProvider).toBe(hub.provider);
        });
    });

    it("reports each step to the footer: resolving, ready, link capture, fallback, waiting", async () => {
        const phases: unknown[] = [];
        await inRoot(async () => {
            const h = mk();
            setMeta("cmd", undefined);
            hub.resolveProviderCli.mockImplementation(async () => {
                phases.push(h.status.launchPhase());
                return { cli_path: "/resolved/claude" };
            });
            hub.logInAndConfirm.mockImplementation(async (p: { onTierChange: (e: unknown) => void }) => {
                phases.push(h.status.launchPhase());
                p.onTierChange({ tier: "fallback" });
                phases.push(h.status.launchPhase());
                p.onTierChange({ tier: "polling", deadlineMs: 123_456 });
                phases.push(h.status.launchPhase());
                return { kind: "signed-in", email: null };
            });
            await h.status.relogin();
        });
        expect(phases[0]).toEqual({ kind: "resolving-cli" });
        expect(phases[1]).toMatchObject({ kind: "waiting-for-login-link" });
        expect(phases[2]).toEqual({ kind: "opening-login-terminal" });
        expect(phases[3]).toEqual({ kind: "waiting-for-login-completion", deadlineMs: 123_456 });
    });

    it("tells the footer a terminal login is opening a terminal, and a no-link relogin too", async () => {
        const seen: unknown[] = [];
        await inRoot(async () => {
            const h = mk();
            hub.logInAndConfirm.mockImplementation(async () => {
                seen.push(h.status.launchPhase());
                return { kind: "signed-in", email: null };
            });
            await h.status.loginViaTerminal();
        });
        expect(seen[0]).toEqual({ kind: "opening-login-terminal" });
        seen.length = 0;
        hub.provider = claude({ headlessLoginUrlUnsupported: true });
        await inRoot(async () => {
            const h = mk();
            hub.logInAndConfirm.mockImplementation(async () => {
                seen.push(h.status.launchPhase());
                return { kind: "signed-in", email: null };
            });
            await h.status.relogin();
        });
        expect(seen[0]).toEqual({ kind: "opening-login-terminal" });
    });

    it("resolves the CLI itself when block meta has none, keeps the path, and logs why", async () => {
        setMeta("cmd", undefined);
        await inRoot(async () => {
            const { status, log } = mk();
            await status.relogin();
            expect(hub.resolveProviderCli).toHaveBeenCalledWith(hub.provider, BLOCK);
            expect(hub.setMeta).toHaveBeenCalledWith(
                { tag: "rpc-client" },
                { oref: `block:${BLOCK}`, meta: { cmd: "/resolved/claude" } }
            );
            expect(loginArgs().cliPath).toBe("/resolved/claude");
            expect(log).toHaveBeenCalledWith(
                "auth",
                "re-login: CLI path not in block meta — resolving it directly",
                "warn"
            );
        });
        hub.logInAndConfirm.mockClear();
        await inRoot(async () => {
            const { status, log } = mk();
            await status.loginViaTerminal();
            expect(log).toHaveBeenCalledWith(
                "auth",
                "login via terminal: CLI path not in block meta — resolving it directly",
                "warn"
            );
        });
    });

    it("goes on with a resolved path even when it can't be saved to block meta", async () => {
        setMeta("cmd", undefined);
        hub.setMeta.mockRejectedValue(new Error("meta down"));
        await inRoot(async () => {
            const { status } = mk();
            await status.relogin();
            expect(loginArgs().cliPath).toBe("/resolved/claude");
        });
    });

    it("stops with a notice and a logged error when the CLI can't be found", async () => {
        setMeta("cmd", undefined);
        for (const [thrown, text] of [
            [new Error("not on PATH"), "not on PATH"],
            ["gone", "gone"],
        ] as const) {
            hub.resolveProviderCli.mockRejectedValue(thrown);
            hub.logInAndConfirm.mockClear();
            await inRoot(async () => {
                const { status, log, onLoginSuccess } = mk();
                await status.relogin();
                const msg = `Couldn't find the claude CLI to run the login: ${text}`;
                expect(log).toHaveBeenCalledWith("auth", msg, "error");
                expect(status.authNotice()).toBe(msg);
                expect(hub.logInAndConfirm).not.toHaveBeenCalled();
                expect(onLoginSuccess).not.toHaveBeenCalled();
                expect(status.loginWaiting()).toBe(false);
            });
        }
    });

    it("uses the env block meta already holds, and builds the isolated one only when it holds none", async () => {
        setMeta("cmd:env", { ONE: "1" });
        await inRoot(async () => {
            const { status } = mk();
            await status.relogin();
            expect(loginArgs().authEnv).toEqual({ ONE: "1" });
            expect(hub.ensureProviderAuthDir).not.toHaveBeenCalled();
        });
        hub.logInAndConfirm.mockClear();
        setMeta("cmd:env", {});
        await inRoot(async () => {
            const { status } = mk();
            await status.relogin();
            expect(loginArgs().authEnv).toEqual({ CLAUDE_CONFIG_DIR: "/auth/claude", EXTRA: "1" });
        });
        hub.logInAndConfirm.mockClear();
        setMeta("cmd:env", "not an object");
        hub.provider = claude({ authConfigDirEnvVar: undefined });
        await inRoot(async () => {
            const { status } = mk();
            await status.relogin();
            expect(loginArgs().authEnv).toEqual({});
        });
    });

    it("says there is no provider and starts nothing", async () => {
        hub.provider = undefined;
        await inRoot(async () => {
            const { status, log } = mk();
            await status.relogin();
            expect(log).toHaveBeenCalledWith("auth", "re-login: no active provider", "warn");
            await status.loginViaTerminal();
            expect(log).toHaveBeenCalledWith("auth", "login via terminal: no active provider", "warn");
            expect(hub.logInAndConfirm).not.toHaveBeenCalled();
            expect(status.loginWaiting()).toBe(false);
        });
    });

    it("refuses a second login while one is in flight, and allows one after", async () => {
        const gate = deferred<{ kind: string; email?: string | null; stage?: string }>();
        hub.logInAndConfirm.mockReturnValueOnce(gate.promise);
        await inRoot(async () => {
            const { status } = mk();
            const first = status.relogin();
            await flush();
            await status.relogin();
            await status.loginViaTerminal();
            expect(hub.windowCalls).toBe(1);
            expect(hub.logInAndConfirm).toHaveBeenCalledTimes(1);
            gate.resolve({ kind: "signed-in", email: null });
            await first;
            await status.relogin();
            expect(hub.logInAndConfirm).toHaveBeenCalledTimes(2);
        });
    });

    it("on success restarts the controller before it tells anyone, releases waiting before the resend, and retries", async () => {
        let waitingAtRecovered: boolean | undefined;
        await inRoot(async () => {
            const h = mk();
            h.onRecovered.mockImplementation(() => {
                hub.order.push("onRecovered");
                waitingAtRecovered = h.status.loginWaiting();
            });
            await h.status.relogin({ retryAfterLogin: true });
            expect(hub.order).toEqual(["registerController", "onLoginSuccess", "onRecovered"]);
            expect(waitingAtRecovered).toBe(false);
            expect(h.onLoginSuccess).toHaveBeenCalledWith("me@x.com");
            expect(h.onReady).not.toHaveBeenCalled();
            expect(h.log).toHaveBeenCalledWith("auth", "Login successful — retrying…");
            expect(h.status.canRetry()).toBe(false);
            expect(h.status.loginWaiting()).toBe(false);
        });
    });

    it("on success without a retry sends the startup sequence instead", async () => {
        await inRoot(async () => {
            const h = mk();
            await h.status.loginViaTerminal({ retryAfterLogin: false });
            expect(h.onReady).toHaveBeenCalledTimes(1);
            expect(h.onRecovered).not.toHaveBeenCalled();
            expect(h.log).toHaveBeenCalledWith("auth", "Login successful");
            expect(h.log).not.toHaveBeenCalledWith("auth", "Login successful — retrying…");
        });
    });

    it("does not retry or announce a login that did not sign in", async () => {
        hub.logInAndConfirm.mockResolvedValue({ kind: "save-failed" });
        await inRoot(async () => {
            const h = mk();
            await h.status.relogin();
            expect(h.onLoginSuccess).not.toHaveBeenCalled();
            expect(h.onRecovered).not.toHaveBeenCalled();
            expect(h.onReady).not.toHaveBeenCalled();
            expect(hub.registerController).not.toHaveBeenCalled();
            expect(h.status.authNotice()).toMatch(/./);
            expect(h.status.loginWaiting()).toBe(false);
        });
    });

    it("logs the terminal login's own notice when nothing was detected, and only then", async () => {
        hub.logInAndConfirm.mockResolvedValue({ kind: "not-detected", stage: "terminal" });
        await inRoot(async () => {
            const h = mk();
            await h.status.loginViaTerminal();
            expect(h.log).toHaveBeenCalledWith(
                "auth",
                "No login detected after 5 minutes. Complete the login in the terminal, then click “Login Again”.",
                "warn"
            );
        });
        await inRoot(async () => {
            const h = mk();
            await h.status.relogin();
            expect(h.log).not.toHaveBeenCalledWith(
                "auth",
                expect.stringContaining("No login detected"),
                expect.anything()
            );
            expect(h.status.authNotice()).toMatch(/terminal window for login/);
        });
        hub.logInAndConfirm.mockResolvedValue({ kind: "unavailable" });
        await inRoot(async () => {
            const h = mk();
            await h.status.loginViaTerminal();
            expect(h.log).not.toHaveBeenCalledWith("auth", expect.any(String), "warn");
            expect(h.status.authNotice()).toBe("Couldn't open a terminal window for login on this platform.");
        });
    });

    it("turns a throw into a logged error and a notice, naming which login failed", async () => {
        for (const [run, label] of [
            [(s: UseAgentControllerStatus) => s.relogin(), "Re-login failed"],
            [(s: UseAgentControllerStatus) => s.loginViaTerminal(), "Terminal login failed"],
        ] as const) {
            for (const [thrown, text] of [
                [new Error("boom"), "boom"],
                ["plain", "plain"],
            ] as const) {
                hub.logInAndConfirm.mockRejectedValue(thrown);
                await inRoot(async () => {
                    const h = mk();
                    await run(h.status);
                    const msg = `${label}: ${text}`;
                    expect(h.log).toHaveBeenCalledWith("auth", msg, "error");
                    expect(h.status.authNotice()).toBe(msg);
                    expect(h.status.loginWaiting()).toBe(false);
                });
            }
        }
    });

    it("cancel while the login runs hides the failure and reports the flow as cancelled", async () => {
        const gate = deferred<{ kind: string; stage: string }>();
        hub.logInAndConfirm.mockReturnValueOnce(gate.promise);
        await inRoot(async () => {
            const h = mk();
            const run = h.status.relogin();
            await flush();
            expect(loginArgs().isCancelled()).toBe(false);
            h.status.cancelLogin();
            expect(loginArgs().isCancelled()).toBe(true);
            expect(h.status.isCancelled()).toBe(true);
            gate.resolve({ kind: "not-detected", stage: "link" });
            await run;
            expect(h.status.authNotice()).toBeNull();
            h.status.resetCancelled();
            expect(h.status.isCancelled()).toBe(false);
        });
    });
});

describe("cancelling and using the terminal instead", () => {
    it("cancel stops the host login, swallows its failure and logs it", async () => {
        const swallow = vi.fn();
        hub.cancel.mockReturnValue({ catch: swallow });
        await inRoot(async () => {
            const { status, log } = mk();
            status.cancelLogin();
            expect(hub.cancel).toHaveBeenCalledTimes(1);
            expect(swallow).toHaveBeenCalledTimes(1);
            expect(log).toHaveBeenCalledWith("auth", "login cancelled", "warn");
            expect(status.isCancelled()).toBe(true);
        });
    });

    it("waits for a running flow to wind down, then starts the terminal login with the flow's own intent", async () => {
        const gate = deferred<{ kind: string; email?: string | null; stage?: string }>();
        hub.logInAndConfirm.mockReturnValueOnce(gate.promise);
        vi.useFakeTimers();
        await inRoot(async () => {
            const h = mk();
            h.status.setAuthNotice("stale");
            const first = h.status.relogin({ retryAfterLogin: false });
            await vi.advanceTimersByTimeAsync(0);
            const switched = h.status.useTerminalInstead();
            await vi.advanceTimersByTimeAsync(600);
            expect(hub.logInAndConfirm).toHaveBeenCalledTimes(1);
            gate.resolve({ kind: "not-detected", stage: "link" });
            await first;
            await vi.advanceTimersByTimeAsync(400);
            await switched;
            expect(hub.logInAndConfirm).toHaveBeenCalledTimes(2);
            expect(hub.logInAndConfirm.mock.calls[1][0].skipTier1).toBe(true);
            expect(h.onReady).toHaveBeenCalledTimes(1);
            expect(h.onRecovered).not.toHaveBeenCalled();
            expect(h.status.authNotice()).toBeNull();
            expect(new Set(hub.sleeps)).toEqual(new Set([200]));
        });
    });

    it("keeps a retry intent through the switch", async () => {
        const gate = deferred<{ kind: string; email?: string | null; stage?: string }>();
        hub.logInAndConfirm.mockReturnValueOnce(gate.promise);
        vi.useFakeTimers();
        await inRoot(async () => {
            const h = mk();
            const first = h.status.relogin({ retryAfterLogin: true });
            await vi.advanceTimersByTimeAsync(0);
            const switched = h.status.useTerminalInstead();
            gate.resolve({ kind: "not-detected", stage: "link" });
            await first;
            await vi.advanceTimersByTimeAsync(400);
            await switched;
            expect(h.onRecovered).toHaveBeenCalledTimes(1);
            expect(h.onReady).not.toHaveBeenCalled();
        });
    });

    it("gives up after twenty seconds with a notice, and starts nothing", async () => {
        const gate = deferred<{ kind: string; email?: string | null; stage?: string }>();
        hub.logInAndConfirm.mockReturnValueOnce(gate.promise);
        vi.useFakeTimers();
        await inRoot(async () => {
            const h = mk();
            const first = h.status.relogin();
            await vi.advanceTimersByTimeAsync(0);
            let done = false;
            const switched = h.status.useTerminalInstead().then(() => {
                done = true;
            });
            await vi.advanceTimersByTimeAsync(19_999);
            expect(done).toBe(false);
            expect(h.status.authNotice()).toBeNull();
            await vi.advanceTimersByTimeAsync(1);
            await switched;
            expect(h.status.authNotice()).toBe(
                "The previous login attempt is taking longer than expected to stop. " +
                    "Please wait a moment and try “Login via terminal” again."
            );
            expect(hub.logInAndConfirm).toHaveBeenCalledTimes(1);
            gate.resolve({ kind: "not-detected", stage: "link" });
            await first;
        });
    });

    it("waits out a /login-driven flow on the shared counter", async () => {
        vi.useFakeTimers();
        await inRoot(async () => {
            const h = mk();
            h.status.beginRecoveryFlow(false);
            const switched = h.status.useTerminalInstead();
            await vi.advanceTimersByTimeAsync(1000);
            expect(hub.logInAndConfirm).not.toHaveBeenCalled();
            h.status.endRecoveryFlow();
            await vi.advanceTimersByTimeAsync(400);
            await switched;
            expect(hub.logInAndConfirm).toHaveBeenCalledTimes(1);
            expect(h.onReady).toHaveBeenCalledTimes(1);
        });
    });

    it("starts the terminal login at once when nothing is in flight", async () => {
        await inRoot(async () => {
            const h = mk();
            await h.status.useTerminalInstead();
            expect(hub.sleeps).toEqual([]);
            expect(hub.logInAndConfirm).toHaveBeenCalledTimes(1);
            expect(h.status.isCancelled()).toBe(false);
        });
    });
});

describe("forcing a controller refresh", () => {
    it("restarts the controller with the pane's size, hands on its status, and reports success", async () => {
        await inRoot(async () => {
            const { status, onControllerStatus } = mk();
            expect(await status.forceControllerRefresh()).toBe(true);
            expect(hub.registerController).toHaveBeenCalledWith(BLOCK, {
                forceRestart: true,
                termSize: { rows: 24, cols: 80 },
            });
            expect(onControllerStatus).toHaveBeenCalledWith({ status: "rts" });
        });
    });

    it("hands on nothing when there is no status, and works without a size or a status handler", async () => {
        hub.registerController.mockResolvedValue(undefined);
        await inRoot(async () => {
            const { status, onControllerStatus } = mk();
            expect(await status.forceControllerRefresh()).toBe(true);
            expect(onControllerStatus).not.toHaveBeenCalled();
        });
        await inRoot(async () => {
            const { status } = mk({ getInitialTermSize: undefined, onControllerStatus: undefined });
            expect(await status.forceControllerRefresh()).toBe(true);
            expect(hub.registerController).toHaveBeenLastCalledWith(BLOCK, { forceRestart: true, termSize: undefined });
        });
    });

    it("reports a failed restart with a warning that says what to do", async () => {
        for (const [thrown, detail] of [
            [new Error("srv down"), "srv down"],
            ["plain", "plain"],
        ] as const) {
            hub.registerController.mockRejectedValue(thrown);
            await inRoot(async () => {
                const { status, log } = mk();
                expect(await status.forceControllerRefresh()).toBe(false);
                expect(log).toHaveBeenCalledWith(
                    "auth",
                    `signed in, but couldn't refresh the running agent with the new login — reopen this pane if it still shows as logged out: ${detail}`,
                    "warn"
                );
            });
        }
    });
});

describe("recovery flows and the controller-healthy signal", () => {
    it("counts each begin and end, and keeps the first intent", async () => {
        await inRoot(async () => {
            const { status } = mk();
            expect(status.loginWaiting()).toBe(false);
            status.beginRecoveryFlow(true);
            status.beginRecoveryFlow(false);
            expect(status.loginWaiting()).toBe(true);
            status.endRecoveryFlow();
            expect(status.loginWaiting()).toBe(true);
            status.endRecoveryFlow();
            expect(status.loginWaiting()).toBe(false);
        });
    });

    it("clears the Log in state and the notice when the controller proves healthy", async () => {
        hub.runLaunchFlow.mockResolvedValue("auth_failed");
        await inRoot(async () => {
            const { status } = mk();
            await status.startLaunchFlow();
            status.setAuthNotice("stale");
            expect(status.canRetry()).toBe(true);
            status.notifyControllerHealthy();
            expect(status.canRetry()).toBe(false);
            expect(status.authNotice()).toBeNull();
        });
    });

    it("sets and clears the auth URL and notice", async () => {
        await inRoot(async () => {
            const { status } = mk();
            status.setAuthUrl("https://x");
            status.setAuthNotice("careful");
            expect(status.authUrl()).toBe("https://x");
            expect(status.authNotice()).toBe("careful");
            status.setAuthUrl(null);
            status.setAuthNotice(null);
            expect(status.authUrl()).toBeNull();
            expect(status.authNotice()).toBeNull();
        });
    });

    it("is loading until the agent is ready, and while the mount flow runs", async () => {
        await inRoot(async () => {
            const { status } = mk();
            expect(status.isLoading()).toBe(true);
            expect(status.agentReady()).toBe(false);
            await status.startLaunchFlow();
            expect(status.isLoading()).toBe(false);
            expect(status.agentReady()).toBe(true);
        });
    });

    it("closing the pane cancels the host login whether or not one is waiting", async () => {
        await createRoot(async (dispose) => {
            mk();
            dispose();
        });
        expect(hub.cancel).toHaveBeenCalledTimes(1);
    });

    it("closing the pane stops later work from changing state", async () => {
        let status!: UseAgentControllerStatus;
        await createRoot(async (dispose) => {
            status = mk().status;
            dispose();
        });
        status.setAuthNotice("late");
        expect(status.authNotice()).toBeNull();
    });
});

describe("rechecking the login after a bind", () => {
    it("says no without a provider, and without a CLI path", async () => {
        hub.provider = undefined;
        await inRoot(async () => {
            expect(await mk().status.recheckAuthAfterBind()).toBe(false);
        });
        hub.provider = claude();
        setMeta("cmd", undefined);
        await inRoot(async () => {
            expect(await mk().status.recheckAuthAfterBind()).toBe(false);
            expect(hub.checkCliAuth).not.toHaveBeenCalled();
        });
    });

    it("runs the provider's check with the pane's env and a ten-second limit, and clears the stale state when it passes", async () => {
        setMeta("cmd:env", { A: "1" });
        hub.runLaunchFlow.mockResolvedValue("auth_failed");
        await inRoot(async () => {
            const { status } = mk();
            await status.startLaunchFlow();
            status.setAuthNotice("stale");
            expect(await status.recheckAuthAfterBind()).toBe(true);
            expect(hub.checkCliAuth).toHaveBeenCalledWith("/bin/claude", "claude auth status", { A: "1" }, 10000);
            expect(status.canRetry()).toBe(false);
            expect(status.authNotice()).toBeNull();
        });
    });

    it("stays blocked when the check does not authenticate or throws", async () => {
        hub.runLaunchFlow.mockResolvedValue("auth_failed");
        for (const mode of ["no", "throw"]) {
            if (mode === "no") hub.checkCliAuth.mockResolvedValue({ authenticated: false });
            else hub.checkCliAuth.mockRejectedValue(new Error("rpc"));
            await inRoot(async () => {
                const { status } = mk();
                await status.startLaunchFlow();
                expect(await status.recheckAuthAfterBind()).toBe(false);
                expect(status.canRetry()).toBe(true);
            });
        }
    });

    it("trusts a container agent's bind and clears the stale state without a host check", async () => {
        setMeta("agentMode", "container");
        hub.runLaunchFlow.mockResolvedValue("auth_failed");
        await inRoot(async () => {
            const { status } = mk();
            await status.startLaunchFlow();
            expect(await status.recheckAuthAfterBind()).toBe(true);
            expect(hub.checkCliAuth).not.toHaveBeenCalled();
            expect(status.canRetry()).toBe(false);
        });
    });
});

describe("binding an account", () => {
    const account = { id: "acct-1", name: "work", display_name: "Work (me@x.com)" } as never;
    const bound = async (over: Partial<UseAgentControllerStatusOptions> = {}) => {
        let h!: ReturnType<typeof mk>;
        await inRoot(async () => {
            h = mk(over);
            await h.status.bindExistingAccount(account);
        });
        return h;
    };

    it("does nothing for a pane with no agent definition", async () => {
        const h = await bound();
        expect(hub.accountSwitch).not.toHaveBeenCalled();
        expect(h.onAccountBound).not.toHaveBeenCalled();
    });

    it("moves the agent to the account after the turn, refreshes the account lists, and names the account", async () => {
        setMeta("agentId", "def-1");
        const h = await bound();
        expect(hub.accountSwitch).toHaveBeenCalledWith(
            { tag: "rpc-client" },
            { agent_id: "def-1", account: "acct-1", when: "after_turn" }
        );
        expect(hub.notifyLinksChanged).toHaveBeenCalledTimes(1);
        expect(h.onAccountBound).toHaveBeenCalledWith("Work (me@x.com)");
        expect(h.status.authNotice()).toBeNull();
        expect(h.status.loginWaiting()).toBe(false);
    });

    it("names the account by its name when it has no display name", async () => {
        setMeta("agentId", "def-1");
        let h!: ReturnType<typeof mk>;
        await inRoot(async () => {
            h = mk();
            await h.status.bindExistingAccount({ id: "acct-1", name: "work", display_name: "" } as never);
        });
        expect(h.onAccountBound).toHaveBeenCalledWith("work");
    });

    it("holds the recovery counter while the switch runs, and clears an old notice first", async () => {
        setMeta("agentId", "def-1");
        const gate = deferred<unknown>();
        hub.accountSwitch.mockReturnValue(gate.promise);
        await inRoot(async () => {
            const { status } = mk();
            status.setAuthNotice("old");
            const run = status.bindExistingAccount(account);
            await flush();
            expect(status.loginWaiting()).toBe(true);
            expect(status.authNotice()).toBeNull();
            gate.resolve({ failed: [], status: "done", account_label: "Work" });
            await run;
            expect(status.loginWaiting()).toBe(false);
        });
    });

    it("says what failed to restart, and does not claim the bind", async () => {
        setMeta("agentId", "def-1");
        hub.accountSwitch.mockResolvedValue({
            failed: ["pane A did not restart", "pane B did not restart"],
            status: "done",
            account_label: "Work",
        });
        const h = await bound();
        expect(h.status.authNotice()).toBe("pane A did not restart pane B did not restart");
        expect(h.onAccountBound).not.toHaveBeenCalled();
    });

    it("says when the move waits for the end of a turn, and still claims the bind", async () => {
        setMeta("agentId", "def-1");
        hub.accountSwitch.mockResolvedValue({ failed: [], status: "queued", account_label: "Work" });
        const h = await bound();
        expect(h.status.authNotice()).toBe("Moves to Work when the turn ends.");
        expect(h.onAccountBound).toHaveBeenCalledTimes(1);
    });

    it("surfaces a failed switch as a notice and a logged error", async () => {
        setMeta("agentId", "def-1");
        for (const [thrown, text] of [
            [new Error("srv said no"), "srv said no"],
            ["plain", "plain"],
        ] as const) {
            hub.accountSwitch.mockRejectedValue(thrown);
            const h = await bound();
            const msg = `Couldn't bind "work": ${text}`;
            expect(h.log).toHaveBeenCalledWith("auth", msg, "error");
            expect(h.status.authNotice()).toBe(msg);
            expect(h.status.loginWaiting()).toBe(false);
            expect(h.onAccountBound).not.toHaveBeenCalled();
        }
    });

    it("starts a container agent's container with the pane's log, and its starter prompt only if the bind answered a sign-in", async () => {
        setMeta("agentId", "def-1");
        setMeta("agentMode", "container");
        let h!: ReturnType<typeof mk>;
        await inRoot(async () => {
            h = mk();
            await h.status.bindExistingAccount(account);
            expect(hub.prestartContainerAgent).toHaveBeenCalledWith(BLOCK, h.log);
            expect(h.onReady).not.toHaveBeenCalled();
            hub.runLaunchFlow.mockResolvedValue("auth_failed");
            await h.status.startLaunchFlow();
            expect(h.status.canRetry()).toBe(true);
            await h.status.bindExistingAccount(account);
            expect(h.onReady).toHaveBeenCalledTimes(1);
            expect(h.status.canRetry()).toBe(false);
        });
    });

    it("starts nothing for a host agent", async () => {
        setMeta("agentId", "def-1");
        setMeta("agentMode", "host");
        hub.runLaunchFlow.mockResolvedValue("auth_failed");
        await inRoot(async () => {
            const h = mk();
            await h.status.startLaunchFlow();
            await h.status.bindExistingAccount(account);
            expect(hub.prestartContainerAgent).not.toHaveBeenCalled();
            expect(h.onReady).not.toHaveBeenCalled();
        });
    });
});

describe("what the hook adds to the pane's flows", () => {
    const sourceOf = (type: string) =>
        getRecentDispatches()
            .filter((r) => r.slice === "launch-phase-state" && (r.command as { type: string }).type === type)
            .map((r) => r.source);

    it("records the user's own clicks as the user's, and the hook's own steps as the system's", async () => {
        await inRoot(async () => {
            const { status } = mk();
            await status.relogin();
            status.cancelLogin();
            expect(sourceOf("LoginRequested")).toEqual(["user"]);
            expect(sourceOf("CancelRequested")).toEqual(["user"]);
            expect(sourceOf("LoginSucceeded")).toEqual(["system"]);
        });
    });

    it("a relogin with no arguments retries the failed turn on success", async () => {
        await inRoot(async () => {
            const h = mk();
            await h.status.relogin();
            expect(h.onRecovered).toHaveBeenCalledTimes(1);
            expect(h.onReady).not.toHaveBeenCalled();
            expect(h.log).toHaveBeenCalledWith("auth", "Login successful — retrying…");
        });
    });

    it("shows the checking-auth phase while the login's env is being built", async () => {
        const gate = deferred<string>();
        hub.ensureProviderAuthDir.mockReturnValue(gate.promise);
        await inRoot(async () => {
            const { status } = mk();
            const run = status.relogin();
            await flush();
            expect(status.launchPhase()).toEqual({ kind: "checking-auth" });
            gate.resolve("/auth/claude");
            await run;
        });
    });

    it("says nothing for a login request that is refused while one runs, even with no provider by then", async () => {
        const gate = deferred<{ kind: string; email?: string | null; stage?: string }>();
        hub.logInAndConfirm.mockReturnValueOnce(gate.promise);
        await inRoot(async () => {
            const { status, log } = mk();
            const first = status.relogin();
            await flush();
            hub.provider = undefined;
            await status.loginViaTerminal();
            expect(log).not.toHaveBeenCalledWith("auth", expect.stringContaining("no active provider"), "warn");
            hub.provider = claude();
            gate.resolve({ kind: "signed-in", email: null });
            await first;
        });
    });

    it("logs nothing as a warning for a relogin or an unavailable terminal that has a notice, nor a terminal login whose notice is empty", async () => {
        hub.logInAndConfirm.mockResolvedValue({ kind: "not-detected", stage: "terminal" });
        await inRoot(async () => {
            const h = mk();
            await h.status.relogin();
            expect(h.log).not.toHaveBeenCalledWith("auth", expect.any(String), "warn");
        });
        hub.logInAndConfirm.mockResolvedValue({ kind: "unavailable" });
        await inRoot(async () => {
            const h = mk();
            await h.status.loginViaTerminal();
            expect(h.log).not.toHaveBeenCalledWith("auth", expect.any(String), "warn");
        });
        hub.logInAndConfirm.mockResolvedValue({ kind: "not-detected", stage: "link" });
        await inRoot(async () => {
            const h = mk();
            await h.status.loginViaTerminal();
            expect(h.log.mock.calls.filter((call) => call[2] === "warn")).toEqual([]);
        });
    });

    it("clears a stale notice before the terminal login even when there is no provider to log in with", async () => {
        hub.provider = undefined;
        await inRoot(async () => {
            const { status } = mk();
            status.setAuthNotice("stale");
            await status.useTerminalInstead();
            expect(status.authNotice()).toBeNull();
        });
    });

    it("is loading while the mount flow runs even after the agent is ready", async () => {
        let loadingAtReady: boolean | undefined;
        await inRoot(async () => {
            const h = mk({
                onReady: () => {
                    loadingAtReady = h.status.isLoading();
                },
            });
            await h.status.startLaunchFlow();
            expect(loadingAtReady).toBe(true);
            expect(h.status.isLoading()).toBe(false);
        });
    });

    it("a recheck for a pane with no provider says no, even for a container agent", async () => {
        hub.provider = undefined;
        setMeta("agentMode", "container");
        await inRoot(async () => {
            expect(await mk().status.recheckAuthAfterBind()).toBe(false);
        });
    });

    it("a pane that has closed starts no launch flow", async () => {
        let status!: UseAgentControllerStatus;
        await createRoot(async (dispose) => {
            status = mk().status;
            dispose();
        });
        await status.startLaunchFlow();
        expect(hub.runLaunchFlow).not.toHaveBeenCalled();
    });
});
