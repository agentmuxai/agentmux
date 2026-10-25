// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for install-sessions: one npm install per provider CLI that outlives
 * the panel that started it (SPEC_AGENT_PICKER_INLINE_CREATE_2026_10_18.md
 * §2.3). A run that ends while no panel shows it says so in a toast; while
 * a panel watches it, the panel shows the end and no toast is pushed.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { pushNotification, invalidateInstallCheck } = vi.hoisted(() => ({
    pushNotification: vi.fn(),
    invalidateInstallCheck: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        InstallStartCommand: vi.fn(),
        InstallCancelCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", async () => {
    const { isNoPayloadMuxEvent } = await vi.importActual<typeof import("@/app/store/mps-events.generated")>("@/app/store/mps-events.generated");
    const mock = { muxEventSubscribe: vi.fn(() => () => {}) };
    // As the real onMuxEvent: a typed subscription is a muxEventSubscribe whose handler gets the data.
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        (mock.muxEventSubscribe as (sub: unknown) => () => void)({ eventType, scope, handler: (e: any) => { if (e?.data != null) handler(e.data, e); else if (isNoPayloadMuxEvent(eventType)) handler(null, e); } } as any);
    return { ...mock, onMuxEvent };
});
vi.mock("@/app/store/flash-notifications", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/store/flash-notifications")>()),
    pushNotification,
}));
vi.mock("./install-check-cache", () => ({ invalidateInstallCheck }));
vi.mock("./defaults/cli-catalog", async (importOriginal) => ({
    ...(await importOriginal<typeof import("./defaults/cli-catalog")>()),
    getCliCatalogEntry: (id: string) => (id === "codex" ? { displayName: "Codex", icon: "", popoverMarkdown: "" } : undefined),
}));
vi.mock("./providers", async (importOriginal) => ({
    ...(await importOriginal<typeof import("./providers")>()),
    getProvider: (id: string) =>
        ({
            codex: { id: "codex", displayName: "Codex", cliCommand: "codex", npmPackage: "@openai/codex", pinnedVersion: "0.116.0" },
            vendor: { id: "vendor", displayName: "Vendor CLI", cliCommand: "vendor", npmPackage: "", pinnedVersion: "" },
        })[id],
}));

import { RpcApi } from "@/app/store/rpc-api";
import { muxEventSubscribe } from "@/app/store/mps";
import {
    __resetAgentInstallSessions,
    agentInstallSession,
    isAgentInstalling,
    onAgentInstallDone,
    watchAgentInstall,
} from "./install-sessions";

let nextSession = 0;

beforeEach(() => {
    vi.mocked(RpcApi.InstallStartCommand).mockImplementation(async () => ({ sessionId: `sess-${++nextSession}` }) as any);
    vi.mocked(RpcApi.InstallCancelCommand).mockResolvedValue({} as any);
});

afterEach(() => {
    vi.clearAllMocks();
    __resetAgentInstallSessions();
});

/** Starts `providerId`'s install and returns a function that ends it. */
async function startRun(providerId = "codex"): Promise<(ok: boolean, error?: string) => void> {
    const session = agentInstallSession(providerId);
    await session.start();
    const sid = session.sessionId();
    expect(sid).toBeTruthy();
    const sub = vi
        .mocked(muxEventSubscribe)
        .mock.calls.map(([s]) => s as unknown as { scope?: string; handler: (e: unknown) => void })
        .find((s) => s.scope === `install:${sid}`);
    expect(sub).toBeTruthy();
    return (ok, error) => sub!.handler({ data: { sessionId: sid, op: "done", ok, ...(error ? { error } : {}) } });
}

describe("install-sessions", () => {
    it("keeps one session per provider", () => {
        expect(agentInstallSession("codex")).toBe(agentInstallSession("codex"));
        expect(agentInstallSession("codex")).not.toBe(agentInstallSession("vendor"));
    });

    it("asks srv to install the provider by id and sends no package", async () => {
        await startRun();
        expect(vi.mocked(RpcApi.InstallStartCommand).mock.calls[0][1]).toEqual({ providerId: "codex" });
    });

    it("refuses a CLI AgentMux doesn't install, without calling the backend", async () => {
        const session = agentInstallSession("vendor");
        await session.start();
        expect(session.state()).toBe("failed");
        expect(RpcApi.InstallStartCommand).not.toHaveBeenCalled();
    });

    it("says whether a provider is installing", async () => {
        expect(isAgentInstalling("codex")).toBe(false);
        const end = await startRun();
        expect(isAgentInstalling("codex")).toBe(true);
        end(true);
        expect(isAgentInstalling("codex")).toBe(false);
    });

    it("toasts a successful run that ends while no panel watches it", async () => {
        const end = await startRun();
        end(true);
        expect(pushNotification).toHaveBeenCalledTimes(1);
        expect(pushNotification.mock.calls[0][0]).toMatchObject({
            title: "Codex is installed",
            type: "info",
            icon: "fa-check",
        });
        // Every picker must stop serving the pre-install answer.
        expect(invalidateInstallCheck).toHaveBeenCalledWith("codex");
    });

    it("toasts a failed run that ends while no panel watches it", async () => {
        const end = await startRun();
        end(false, "npm exited Some(1)");
        expect(pushNotification).toHaveBeenCalledTimes(1);
        expect(pushNotification.mock.calls[0][0]).toMatchObject({ title: "Codex didn't install", type: "error" });
        expect(invalidateInstallCheck).toHaveBeenCalledWith("codex");
    });

    it("pushes no toast while a panel watches the run", async () => {
        const release = watchAgentInstall("codex");
        const end = await startRun();
        end(true);
        expect(invalidateInstallCheck).toHaveBeenCalledWith("codex");
        expect(pushNotification).not.toHaveBeenCalled();
        release();
    });

    it("toasts again once the last panel stops watching, and a double release counts once", async () => {
        const releaseA = watchAgentInstall("codex");
        const releaseB = watchAgentInstall("codex");
        releaseA();
        releaseA();
        let end = await startRun();
        end(true);
        // B still watches.
        expect(pushNotification).not.toHaveBeenCalled();

        releaseB();
        end = await startRun();
        end(true);
        expect(pushNotification).toHaveBeenCalledTimes(1);
    });

    it("tells done listeners how a run ended, until they unsubscribe", async () => {
        const cb = vi.fn();
        const unsubscribe = onAgentInstallDone("codex", cb);
        let end = await startRun();
        end(true);
        expect(cb).toHaveBeenCalledWith(true);

        end = await startRun();
        end(false, "boom");
        expect(cb).toHaveBeenLastCalledWith(false);

        unsubscribe();
        end = await startRun();
        end(true);
        expect(cb).toHaveBeenCalledTimes(2);
    });

    it("cancels a running install only when asked", async () => {
        await startRun();
        const session = agentInstallSession("codex");
        await session.cancel();
        expect(RpcApi.InstallCancelCommand).toHaveBeenCalledWith(expect.anything(), { sessionId: session.sessionId() });
    });

    it("forgets every session on reset", () => {
        const before = agentInstallSession("codex");
        __resetAgentInstallSessions();
        expect(agentInstallSession("codex")).not.toBe(before);
    });
});
