// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { hostCallTiming, ipcSplit, recordIpcRoundtrip } from "./index";

afterEach(() => vi.restoreAllMocks());

describe("recordIpcRoundtrip", () => {
    // A slow-IPC warning is a console line, and console lines go to the host
    // over these commands: warning about them would feed the log pipe itself.
    it.each(["fe_log_structured", "fe_log", "fe_log_batch"])("never warns about the log pipe's own %s", (cmd) => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        recordIpcRoundtrip(cmd, 500);
        expect(warn).not.toHaveBeenCalled();
    });

    it("warns about any other command over a frame", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        recordIpcRoundtrip("browser_panes_set_rects", 40);
        expect(warn).toHaveBeenCalledWith(expect.stringContaining("browser_panes_set_rects"));
    });

    it("keeps the line as it was without the host's timing", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        recordIpcRoundtrip("input_select", 989.04);
        expect(warn).toHaveBeenCalledWith("[perf] ipc input_select 989.0ms");
    });

    // The three cases a slow call can be: it waited to reach the host, the host was slow, or the answer
    // waited for this page.
    it("says where a slow call's time went when the host reported its side", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        recordIpcRoundtrip("set_taskbar_attention", 1392, { waited_ms: 1385, handled_ms: 2 });
        recordIpcRoundtrip("input_select", 989, { waited_ms: 1, handled_ms: 980 });
        recordIpcRoundtrip("browser_panes_set_overlay_clip", 1520, { waited_ms: 3, handled_ms: 4 });
        expect(warn.mock.calls.map((c) => c[0])).toEqual([
            "[perf] ipc set_taskbar_attention 1392.0ms (to host 1385ms, host 2ms, back 5ms)",
            "[perf] ipc input_select 989.0ms (to host 1ms, host 980ms, back 8ms)",
            "[perf] ipc browser_panes_set_overlay_clip 1520.0ms (to host 3ms, host 4ms, back 1513ms)",
        ]);
    });

    it("says nothing about a call inside a frame, whatever the host reported", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        recordIpcRoundtrip("get_platform", 12, { waited_ms: 1, handled_ms: 0 });
        expect(warn).not.toHaveBeenCalled();
    });
});

describe("ipcSplit", () => {
    it("names only the host's time when the host could not tell the wait", () => {
        expect(ipcSplit(500, { handled_ms: 480 })).toBe("host 480ms");
    });

    it("never shows a negative way back when the two clocks disagree by a few milliseconds", () => {
        expect(ipcSplit(100, { waited_ms: 60, handled_ms: 45 })).toBe("to host 60ms, host 45ms, back 0ms");
    });
});

describe("hostCallTiming", () => {
    it("reads the timing a host answer carries", () => {
        expect(hostCallTiming({ success: true, data: 1, timing: { waited_ms: 3, handled_ms: 7 } })).toEqual({ waited_ms: 3, handled_ms: 7 });
        expect(hostCallTiming({ success: true, timing: { handled_ms: 7 } })).toEqual({ handled_ms: 7 });
    });

    it("is undefined for an answer with none (an older host) or one that is not this shape", () => {
        expect(hostCallTiming({ success: true, data: 1 })).toBeUndefined();
        expect(hostCallTiming({ success: true, timing: { waited_ms: 3 } })).toBeUndefined();
        expect(hostCallTiming({ success: true, timing: "fast" })).toBeUndefined();
        expect(hostCallTiming(null)).toBeUndefined();
    });
});
