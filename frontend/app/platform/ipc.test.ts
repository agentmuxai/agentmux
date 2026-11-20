// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const recordIpcRoundtrip = vi.fn();
vi.mock("@/perf", async (original) => ({ ...(await original<typeof import("@/perf")>()), recordIpcRoundtrip: (...a: unknown[]) => recordIpcRoundtrip(...a) }));

import { invokeCommand } from "./ipc";

const answer = (body: unknown, status = 200) => ({ status, ok: status >= 200 && status < 300, statusText: "", json: async () => body }) as Response;

describe("invokeCommand and the host's timing", () => {
    let fetchMock: ReturnType<typeof vi.fn>;

    beforeEach(() => {
        recordIpcRoundtrip.mockClear();
        fetchMock = vi.fn();
        vi.stubGlobal("fetch", fetchMock);
        vi.stubGlobal("window", { __AGENTMUX_IPC_PORT__: 4321, __AGENTMUX_IPC_TOKEN__: "token" });
    });

    afterEach(() => {
        vi.unstubAllGlobals();
        vi.useRealTimers();
    });

    it("sends the time of the send with each call", async () => {
        vi.useFakeTimers();
        vi.setSystemTime(1_795_000_000_000);
        fetchMock.mockResolvedValue(answer({ success: true, data: "win32" }));
        await invokeCommand("get_platform");
        const [url, init] = fetchMock.mock.calls[0];
        expect(url).toBe("http://127.0.0.1:4321/ipc");
        expect(JSON.parse(init.body)).toEqual({ cmd: "get_platform", args: {}, sent_ms: 1_795_000_000_000 });
    });

    it("hands the host's side of the call to the recorder", async () => {
        fetchMock.mockResolvedValue(answer({ success: true, data: true, timing: { waited_ms: 1385, handled_ms: 2 } }));
        await expect(invokeCommand("set_taskbar_attention", { count: 1 })).resolves.toBe(true);
        expect(recordIpcRoundtrip).toHaveBeenCalledWith("set_taskbar_attention", expect.any(Number), { waited_ms: 1385, handled_ms: 2 });
    });

    it("records a call an older host answered, with no split", async () => {
        fetchMock.mockResolvedValue(answer({ success: true, data: 1 }));
        await invokeCommand("get_zoom_factor");
        expect(recordIpcRoundtrip).toHaveBeenCalledWith("get_zoom_factor", expect.any(Number), undefined);
    });

    it("gives no split for a call that was answered only after a retry", async () => {
        vi.useFakeTimers();
        fetchMock.mockResolvedValueOnce(answer({}, 401)).mockResolvedValueOnce(answer({ success: true, data: 1, timing: { waited_ms: 1, handled_ms: 1 } }));
        const call = invokeCommand("get_zoom_factor");
        await vi.advanceTimersByTimeAsync(300);
        await call;
        expect(fetchMock).toHaveBeenCalledTimes(2);
        expect(recordIpcRoundtrip).toHaveBeenCalledWith("get_zoom_factor", expect.any(Number), undefined);
    });
});
