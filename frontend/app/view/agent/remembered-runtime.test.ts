// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const call = vi.fn();
// A rejection is produced by a plain function, not by the vi.fn: vitest leaves
// the promise it records for a rejecting mock unhandled and fails the test.
let failWith: string | null = null;
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentLastRuntimeCommand: (_c: unknown, data: unknown) =>
            failWith ? Promise.reject(new Error(failWith)) : call(data),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { parseRememberedRuntime, pickedRuntime, recallRuntime, rememberRuntime } from "./remembered-runtime";

const MODELS = ["sonnet", "opus", "haiku"];

describe("parseRememberedRuntime", () => {
    it("keeps valid settings", () => {
        expect(parseRememberedRuntime('{"model":"opus","effort":"xhigh","permissionMode":"default"}', MODELS)).toEqual({
            model: "opus",
            effort: "xhigh",
            permissionMode: "default",
        });
    });
    it("drops, setting by setting, what is no longer a choice", () => {
        // an agent moved to a provider whose catalog has no "opus"; an effort that does not exist
        expect(parseRememberedRuntime('{"model":"opus","effort":"ludicrous","permissionMode":"plan"}', ["gpt-5"])).toEqual({
            permissionMode: "plan",
        });
    });
    it("drops a model when the provider has no catalog", () => {
        expect(parseRememberedRuntime('{"model":"opus"}', undefined)).toEqual({});
    });
    it("is empty for nothing, garbage, or the wrong shape", () => {
        for (const raw of [undefined, null, "", "not json", "[]", '"opus"', "5", "null"]) {
            expect(parseRememberedRuntime(raw as any, MODELS), String(raw)).toEqual({});
        }
    });
    it("ignores values of the wrong type", () => {
        expect(parseRememberedRuntime('{"model":5,"effort":["high"]}', MODELS)).toEqual({});
    });
});

describe("pickedRuntime", () => {
    it("holds only what was picked", () => {
        expect(pickedRuntime({ effort: "max" })).toEqual({ effort: "max" });
        expect(pickedRuntime({ model: "haiku", permissionMode: "plan" })).toEqual({ model: "haiku", permissionMode: "plan" });
    });
    it("is null for a restart with no pick", () => {
        expect(pickedRuntime({})).toBeNull();
    });
});

describe("rememberRuntime", () => {
    beforeEach(() => {
        call.mockReset();
        failWith = null;
    });

    it("writes only the picked settings over what was remembered", async () => {
        call.mockResolvedValueOnce({ runtime: '{"model":"opus","permissionMode":"plan"}' }).mockResolvedValueOnce({ runtime: "" });
        await rememberRuntime("agent-1", { effort: "max" });
        expect(call).toHaveBeenNthCalledWith(1, { id: "agent-1" });
        const written = JSON.parse(call.mock.calls[1][0].runtime);
        expect(written).toEqual({ model: "opus", permissionMode: "plan", effort: "max" });
        expect(call.mock.calls[1][0].id).toBe("agent-1");
    });

    it("writes nothing for a restart with no pick, or with no agent", async () => {
        await rememberRuntime("agent-1", {});
        await rememberRuntime(undefined, { model: "opus" });
        expect(call).not.toHaveBeenCalled();
    });

    it("replaces an unreadable stored value", async () => {
        call.mockResolvedValueOnce({ runtime: "{{garbage" }).mockResolvedValueOnce({ runtime: "" });
        await rememberRuntime("agent-1", { model: "haiku" });
        expect(JSON.parse(call.mock.calls[1][0].runtime)).toEqual({ model: "haiku" });
    });

    it("never throws: a failure to remember must not fail the pick", async () => {
        failWith = "srv down";
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        await expect(rememberRuntime("agent-1", { model: "opus" })).resolves.toBeUndefined();
        warn.mockRestore();
    });
});

describe("recallRuntime", () => {
    beforeEach(() => {
        call.mockReset();
        failWith = null;
    });

    it("returns the validated remembered settings", async () => {
        call.mockResolvedValue({ runtime: '{"model":"opus","effort":"xhigh"}' });
        expect(await recallRuntime("agent-1", MODELS)).toEqual({ model: "opus", effort: "xhigh" });
    });
    it("is empty with no agent or on error", async () => {
        expect(await recallRuntime(undefined, MODELS)).toEqual({});
        failWith = "nope";
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        expect(await recallRuntime("agent-1", MODELS)).toEqual({});
        warn.mockRestore();
    });
});
