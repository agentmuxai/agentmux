// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({ InstallCheckCommand: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { checkProviderInstalled, invalidateInstallCheck, onInstallCheckInvalidated } from "./install-check-cache";

const claude = { providerId: "claude", cliCommand: "claude", npmPackage: "@anthropic-ai/claude-code" };
const codex = { providerId: "codex", cliCommand: "codex", npmPackage: "@openai/codex" };
const ok = { installed: true, version: "1.0.0" };

let now = 0;
const clock = () => now;

beforeEach(() => {
    invalidateInstallCheck();
    rpc.InstallCheckCommand.mockReset().mockResolvedValue(ok);
    now = 1_000;
});

describe("install-check cache", () => {
    it("shares one in-flight check between every caller for the same CLI", async () => {
        const all = await Promise.all(Array.from({ length: 40 }, () => checkProviderInstalled(claude, clock)));
        expect(rpc.InstallCheckCommand).toHaveBeenCalledTimes(1);
        expect(all.every((r) => r === ok)).toBe(true);
    });

    it("checks each provider CLI separately", async () => {
        await Promise.all([checkProviderInstalled(claude, clock), checkProviderInstalled(codex, clock)]);
        expect(rpc.InstallCheckCommand).toHaveBeenCalledTimes(2);
    });

    it("reuses an answer for 30 s, then asks again", async () => {
        await checkProviderInstalled(claude, clock);
        now += 29_000;
        await checkProviderInstalled(claude, clock);
        expect(rpc.InstallCheckCommand).toHaveBeenCalledTimes(1);
        now += 2_000;
        await checkProviderInstalled(claude, clock);
        expect(rpc.InstallCheckCommand).toHaveBeenCalledTimes(2);
    });

    it("asks again after the provider's CLI was installed, and only for that provider", async () => {
        await Promise.all([checkProviderInstalled(claude, clock), checkProviderInstalled(codex, clock)]);
        invalidateInstallCheck("claude");
        await Promise.all([checkProviderInstalled(claude, clock), checkProviderInstalled(codex, clock)]);
        expect(rpc.InstallCheckCommand).toHaveBeenCalledTimes(3);
        expect(rpc.InstallCheckCommand.mock.calls[2][1]).toEqual(claude);
    });

    it("never caches a failed check", async () => {
        rpc.InstallCheckCommand.mockRejectedValueOnce(new Error("rpc down"));
        await expect(checkProviderInstalled(claude, clock)).rejects.toThrow("rpc down");
        await expect(checkProviderInstalled(claude, clock)).resolves.toBe(ok);
        expect(rpc.InstallCheckCommand).toHaveBeenCalledTimes(2);
    });

    it("tells subscribers which provider was forgotten, until they unsubscribe", () => {
        const seen: (string | undefined)[] = [];
        const off = onInstallCheckInvalidated((id) => seen.push(id));
        invalidateInstallCheck("grok");
        invalidateInstallCheck();
        off();
        invalidateInstallCheck("codex");
        expect(seen).toEqual(["grok", undefined]);
    });

    it("gives a caller the newer answer when its check was forgotten while in flight", async () => {
        let answerOld!: (r: typeof ok) => void;
        rpc.InstallCheckCommand.mockImplementationOnce(() => new Promise((res) => (answerOld = res)));
        const first = checkProviderInstalled(claude, clock);
        // An install finished: the answer in flight is stale, and a new check finds the CLI.
        invalidateInstallCheck("claude");
        const fresh = { installed: true, version: "2.0.0" };
        rpc.InstallCheckCommand.mockResolvedValueOnce(fresh);
        const second = checkProviderInstalled(claude, clock);
        await expect(second).resolves.toBe(fresh);
        // The old check answers last, saying "not installed": its caller still gets the newer answer.
        answerOld({ installed: false, version: null } as unknown as typeof ok);
        await expect(first).resolves.toBe(fresh);
    });

    it("gives a caller the newer answer when its forgotten check fails late", async () => {
        let failOld!: (e: Error) => void;
        rpc.InstallCheckCommand.mockImplementationOnce(() => new Promise((_res, rej) => (failOld = rej)));
        const first = checkProviderInstalled(claude, clock);
        invalidateInstallCheck("claude");
        const fresh = { installed: true, version: "2.0.0" };
        rpc.InstallCheckCommand.mockResolvedValueOnce(fresh);
        await expect(checkProviderInstalled(claude, clock)).resolves.toBe(fresh);
        failOld(new Error("timed out"));
        await expect(first).resolves.toBe(fresh);
    });

    it("still fails its callers when the current check fails", async () => {
        rpc.InstallCheckCommand.mockRejectedValueOnce(new Error("backend down"));
        await expect(checkProviderInstalled(claude, clock)).rejects.toThrow("backend down");
    });
});
