// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The srv side of the login backend (SPEC_PROVIDER_LOGIN_THROUGH_SRV_2026_10_09.md
// L4): the flow's primitives mapped onto srv's auth.* commands, one login at a
// time; and which backend a host gets.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const createBlock = vi.hoisted(() => vi.fn(async () => "block-1"));
vi.mock("@/app/store/global", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/store/global")>()),
    createBlock,
}));

import { __test, loginBackend, type LoginStart } from "./login-backend";

const START: LoginStart = {
    providerId: "claude",
    cliPath: "/bin/claude",
    loginArgs: ["auth", "login"],
    checkArgs: ["auth", "status"],
    authEnv: { CLAUDE_CONFIG_DIR: "/accounts/a" },
    requiresTty: true,
    authConfigDirEnvVar: "CLAUDE_CONFIG_DIR",
};

function fakeRpc(polls: Array<Record<string, unknown>> = [], startUrl?: string) {
    let n = 0;
    const rpc = {
        AuthStartCommand: vi.fn(async () => ({ sessionId: `s${++n}`, authUrl: startUrl })),
        AuthPollCommand: vi.fn(async () => polls.shift() ?? { status: "pending" }),
        AuthSubmitCallbackCommand: vi.fn(async () => ({ success: true })),
        AuthCancelCommand: vi.fn(async () => ({})),
    };
    return { rpc, backend: __test.makeSrvBackend(rpc as any, {} as any) };
}

describe("srv login backend", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("starts through auth.start with the flow's own account dir, and returns its URL", async () => {
        const { rpc, backend } = fakeRpc([], "https://auth.example/x");
        await expect(backend.start(START)).resolves.toEqual({ url: "https://auth.example/x" });
        expect(rpc.AuthStartCommand).toHaveBeenCalledWith(expect.anything(), {
            providerId: "claude",
            cliPath: "/bin/claude",
            authLoginArgs: ["auth", "login"],
            authCheckArgs: ["auth", "status"],
            authEnv: { CLAUDE_CONFIG_DIR: "/accounts/a" },
            requiresTty: true,
        });
    });

    it("polls for a URL that appears later, a device code's included", async () => {
        const { backend } = fakeRpc([
            { status: "pending" },
            { status: "code-emitted", verificationUrl: "https://dev/x", deviceCode: "ABCD-1234" },
        ]);
        const url = backend.start(START);
        await vi.advanceTimersByTimeAsync(1_000);
        await expect(url).resolves.toEqual({ url: "https://dev/x", deviceCode: "ABCD-1234" });
    });

    it("gives up with null when no URL appears in the capture window", async () => {
        const { backend } = fakeRpc();
        const url = backend.start(START);
        await vi.advanceTimersByTimeAsync(16_000);
        await expect(url).resolves.toBeNull();
    });

    it("a new start cancels the running one and bumps the generation", async () => {
        const { rpc, backend } = fakeRpc([], "https://u");
        await backend.start(START);
        const first = (await backend.status()).generation;
        await backend.start(START);
        expect(rpc.AuthCancelCommand).toHaveBeenCalledWith(expect.anything(), { sessionId: "s1" });
        expect((await backend.status()).generation).toBe(first + 1);
    });

    it("of two overlapping starts, the later one owns the slot and the earlier one's session is cancelled", async () => {
        const { rpc, backend } = fakeRpc([], "https://u");
        let releaseFirst!: () => void;
        rpc.AuthStartCommand.mockImplementationOnce(
            () =>
                new Promise((resolve) => {
                    releaseFirst = () => resolve({ sessionId: "slow", authUrl: "https://slow" });
                }),
        );
        const first = backend.start(START);
        await expect(backend.start(START)).resolves.toEqual({ url: "https://u" });
        releaseFirst();
        await expect(first).resolves.toEqual({ superseded: true });
        expect(rpc.AuthCancelCommand).toHaveBeenCalledWith(expect.anything(), { sessionId: "slow" });
        await backend.submitCode("claude", "abc");
        expect(rpc.AuthSubmitCallbackCommand).toHaveBeenCalledWith(expect.anything(), {
            sessionId: "s1",
            callbackUrl: "abc",
        });
    });

    it("a cancel while srv is still starting the login gives that session up", async () => {
        const { rpc, backend } = fakeRpc([], "https://u");
        let release!: () => void;
        rpc.AuthStartCommand.mockImplementationOnce(
            () => new Promise((resolve) => (release = () => resolve({ sessionId: "late", authUrl: "https://late" }))),
        );
        const started = backend.start(START);
        await backend.cancel();
        release();
        await expect(started).resolves.toEqual({ superseded: true });
        expect(rpc.AuthCancelCommand).toHaveBeenCalledWith(expect.anything(), { sessionId: "late" });
        expect(await backend.status()).toMatchObject({ active: false });
    });

    it("a URL from a poll that a newer start overtook isn't returned", async () => {
        const { rpc, backend } = fakeRpc();
        let releasePoll!: () => void;
        rpc.AuthPollCommand.mockImplementationOnce(
            () => new Promise((resolve) => (releasePoll = () => resolve({ status: "url-available", authUrl: "https://old" }))),
        );
        const first = backend.start(START);
        await vi.advanceTimersByTimeAsync(500); // the first poll is now in flight
        rpc.AuthStartCommand.mockResolvedValueOnce({ sessionId: "s2", authUrl: "https://new" });
        await expect(backend.start(START)).resolves.toEqual({ url: "https://new" });
        releasePoll();
        await expect(first).resolves.toEqual({ superseded: true });
        await backend.submitCode("claude", "abc");
        expect(rpc.AuthSubmitCallbackCommand).toHaveBeenCalledWith(expect.anything(), { sessionId: "s2", callbackUrl: "abc" });
    });

    it("delivers a pasted code to the running session", async () => {
        const { rpc, backend } = fakeRpc([], "https://u");
        await expect(backend.submitCode("claude", "abc")).rejects.toThrow(/no sign-in/);
        await backend.start(START);
        await backend.submitCode("claude", "abc");
        expect(rpc.AuthSubmitCallbackCommand).toHaveBeenCalledWith(expect.anything(), {
            sessionId: "s1",
            callbackUrl: "abc",
        });
        rpc.AuthSubmitCallbackCommand.mockResolvedValueOnce({ success: false, error: "bad code" } as any);
        await expect(backend.submitCode("claude", "x")).rejects.toThrow("bad code");
    });

    it("reports active until the session ends, and credentials only on success", async () => {
        const { backend } = fakeRpc([{ status: "url-available" }, { status: "success" }, { status: "failed" }], "https://u");
        expect(await backend.status()).toMatchObject({ active: false, credential_changed: false });
        await backend.start(START);
        expect(await backend.status()).toMatchObject({ active: true, credential_changed: false });
        expect(await backend.status()).toMatchObject({ active: false, credential_changed: true });
        expect(await backend.status()).toMatchObject({ active: false, credential_changed: false });
    });

    it("cancel stops the session once; status is idle after", async () => {
        const { rpc, backend } = fakeRpc([], "https://u");
        await backend.start(START);
        await backend.cancel();
        await backend.cancel();
        expect(rpc.AuthCancelCommand).toHaveBeenCalledTimes(1);
        expect(await backend.status()).toMatchObject({ active: false });
    });
});

describe("srv login terminal (L3)", () => {
    it("opens a terminal pane that srv runs the login in, with the account's environment", async () => {
        const { backend } = fakeRpc();
        await backend.openTerminal("/bin/claude", ["auth", "login"], { CLAUDE_CONFIG_DIR: "/accounts/a" });
        expect(createBlock).toHaveBeenCalledWith({
            meta: expect.objectContaining({
                view: "term",
                controller: "cmd",
                cmd: "/bin/claude",
                "cmd:args": ["auth", "login"],
                "cmd:env": { CLAUDE_CONFIG_DIR: "/accounts/a" },
                "cmd:interactive": true,
                "cmd:runonstart": true,
                "cmd:closeonexit": true,
            }),
        });
    });
});

describe("loginBackend()", () => {
    const original = window.api;
    afterEach(() => {
        window.api = original;
    });

    it("is the host's unless the host says it has no login", () => {
        window.api = undefined as any;
        expect(loginBackend()).toBe(__test.hostBackend);
        window.api = { getHostCaps: () => ({ hostLogin: true }) } as any;
        expect(loginBackend()).toBe(__test.hostBackend);
        window.api = { getHostCaps: () => ({ hostLogin: false }) } as any;
        expect(loginBackend()).not.toBe(__test.hostBackend);
    });
});
