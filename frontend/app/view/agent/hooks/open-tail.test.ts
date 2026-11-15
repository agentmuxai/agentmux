// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} as Record<string, unknown> }));

import { RpcApi } from "@/app/store/rpc-api";
import { __resetOpenTailForTests, COUNT_GIVE_UP_MS, COUNT_POLL_MS, readOpenTail, startsAtTurn, usableOpenTail, type OpenTailResult } from "./open-tail";

const good = (over: Partial<OpenTailResult> = {}): OpenTailResult => ({
    lines: ["a", "b", "c"],
    stream: "b:blk-1",
    gen: "g1",
    start_line: 7,
    end_line: 10,
    total_lines: 10,
    ...over,
});

const REQ = { block_id: "blk-1", filename: "output" as const };

beforeEach(() => __resetOpenTailForTests());

describe("usableOpenTail", () => {
    it("accepts a slice with consistent line positions", () => {
        expect(usableOpenTail(good())).toBe(true);
        expect(usableOpenTail(good({ lines: [], start_line: 0, end_line: 0, total_lines: 0 }))).toBe(true);
    });

    it("rejects a file srv has no line counter for (no stream or generation)", () => {
        expect(usableOpenTail(good({ stream: undefined }))).toBe(false);
        expect(usableOpenTail(good({ gen: undefined }))).toBe(false);
        expect(usableOpenTail(good({ gen: "" }))).toBe(false);
    });

    it("rejects positions that don't add up: end_line must be start_line + lines", () => {
        expect(usableOpenTail(good({ end_line: 11 }))).toBe(false);
        expect(usableOpenTail(good({ start_line: -1, end_line: 2 }))).toBe(false);
        expect(usableOpenTail(good({ start_line: undefined }))).toBe(false);
    });

    it("rejects a total that is below the slice's end", () => {
        expect(usableOpenTail(good({ total_lines: 9 }))).toBe(false);
    });

    it("rejects stamps that don't run parallel to the lines", () => {
        expect(usableOpenTail(good({ stamps: [1, 2] }))).toBe(false);
        expect(usableOpenTail(good({ stamps: [1, 2, 3] }))).toBe(true);
    });

    it("rejects nothing-at-all", () => {
        expect(usableOpenTail(null)).toBe(false);
        expect(usableOpenTail(undefined)).toBe(false);
    });
});

describe("readOpenTail", () => {
    it("makes no call at all while srv has no generated command for it", async () => {
        expect((RpcApi as Record<string, unknown>).AgentOpenTailCommand).toBeUndefined();
        expect(await readOpenTail(REQ)).toBeNull();
    });

    it("calls the generated command once srv has it", async () => {
        const stub = vi.fn().mockResolvedValue(good());
        (RpcApi as Record<string, unknown>).AgentOpenTailCommand = stub;
        try {
            expect(await readOpenTail(REQ)).toMatchObject({ end_line: 10 });
            expect(stub).toHaveBeenCalledWith({}, REQ, { timeout: 5000 });
        } finally {
            delete (RpcApi as Record<string, unknown>).AgentOpenTailCommand;
        }
    });

    it("returns a usable answer as it is", async () => {
        const call = vi.fn().mockResolvedValue(good());
        expect(await readOpenTail(REQ, { call })).toMatchObject({ start_line: 7, end_line: 10, gen: "g1" });
        expect(call).toHaveBeenCalledWith(REQ);
    });

    it("returns null for an answer without usable line positions, so the pane opens as before", async () => {
        expect(await readOpenTail(REQ, { call: vi.fn().mockResolvedValue(good({ stream: undefined })) })).toBeNull();
    });

    it("returns null when the read fails, and tries again on the next open", async () => {
        const call = vi.fn().mockRejectedValueOnce(new Error("timeout")).mockResolvedValue(good());
        expect(await readOpenTail(REQ, { call })).toBeNull();
        expect(await readOpenTail(REQ, { call })).not.toBeNull();
        expect(call).toHaveBeenCalledTimes(2);
    });

    it("asks again while srv counts the transcript, and returns the answer once it has line positions", async () => {
        const counting: OpenTailResult = { lines: [], counting: true, size: 3_000_000_000 };
        const call = vi.fn().mockResolvedValueOnce(counting).mockResolvedValueOnce(counting).mockResolvedValueOnce(good());
        const sleep = vi.fn().mockResolvedValue(undefined);
        expect(await readOpenTail(REQ, { call, sleep })).toMatchObject({ end_line: 10 });
        expect(call).toHaveBeenCalledTimes(3);
        expect(sleep).toHaveBeenCalledTimes(2);
        expect(sleep).toHaveBeenCalledWith(COUNT_POLL_MS);
    });

    it("stops asking when the pane is gone", async () => {
        const call = vi.fn().mockResolvedValue({ lines: [], counting: true });
        let gone = false;
        const sleep = vi.fn(async () => {
            gone = true;
        });
        expect(await readOpenTail(REQ, { call, sleep, stopped: () => gone })).toBeNull();
        expect(call).toHaveBeenCalledTimes(1);
    });

    it("gives up after two minutes of counting, so the pane opens the usual way", async () => {
        const call = vi.fn().mockResolvedValue({ lines: [], counting: true });
        let t = 0;
        const sleep = vi.fn(async (ms: number) => {
            t += ms;
        });
        expect(await readOpenTail(REQ, { call, sleep, now: () => t })).toBeNull();
        expect(t).toBeGreaterThanOrEqual(COUNT_GIVE_UP_MS);
        expect(call.mock.calls.length).toBeLessThanOrEqual(COUNT_GIVE_UP_MS / COUNT_POLL_MS + 2);
    });

    it("does not wait for an answer that is not counting and has no positions", async () => {
        const call = vi.fn().mockResolvedValue({ lines: [], size: 5 });
        const sleep = vi.fn();
        expect(await readOpenTail(REQ, { call, sleep })).toBeNull();
        expect(call).toHaveBeenCalledTimes(1);
        expect(sleep).not.toHaveBeenCalled();
    });

    it("stops asking once srv says it doesn't know the command", async () => {
        const call = vi.fn().mockRejectedValue(new Error("unknown command: agent:open_tail"));
        expect(await readOpenTail(REQ, { call })).toBeNull();
        expect(await readOpenTail(REQ, { call })).toBeNull();
        expect(await readOpenTail(REQ, { call })).toBeNull();
        expect(call).toHaveBeenCalledTimes(1);
    });
});

describe("startsAtTurn", () => {
    it("is true for a user message: no streamed run crosses it", () => {
        expect(startsAtTurn('{"type":"user","message":{"role":"user","content":"hello"}}')).toBe(true);
        expect(startsAtTurn('{"type":"user","message":{"content":[{"type":"text","text":"hi"}]}}')).toBe(true);
    });

    it("is false for a tool's result, which answers a call from the turn before it", () => {
        expect(startsAtTurn('{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"x"}]}}')).toBe(false);
    });

    it("is false for a subagent's message", () => {
        expect(startsAtTurn('{"type":"user","parent_tool_use_id":"toolu_1","message":{"content":"x"}}')).toBe(false);
    });

    it("is false for anything that may be the middle of a response", () => {
        expect(startsAtTurn('{"type":"stream_event","event":{"type":"content_block_delta","delta":{"text":"par"}}}')).toBe(false);
        expect(startsAtTurn('{"type":"assistant","message":{"content":[{"type":"text","text":"part"}]}}')).toBe(false);
        expect(startsAtTurn("not json")).toBe(false);
        expect(startsAtTurn("")).toBe(false);
        expect(startsAtTurn(undefined)).toBe(false);
    });
});
