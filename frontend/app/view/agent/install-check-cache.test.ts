// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({ InstallCheckCommand: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { checkProviderInstalled, invalidateInstallCheck } from "./install-check-cache";

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
});
