// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { createLongFrameLogger, describeLongFrame, startLongFrameObserver, type LongFrame } from "./long-frames";

afterEach(() => vi.unstubAllGlobals());

const frame = (over: Partial<LongFrame> = {}): LongFrame => ({ startTime: 1000, duration: 180, ...over });

describe("describeLongFrame", () => {
    it("names the slowest scripts, who invoked them and where, and how the frame divides", () => {
        const line = describeLongFrame(
            frame({
                blockingDuration: 130,
                renderStart: 1140,
                styleAndLayoutStart: 1150,
                scripts: [
                    { duration: 20, invokerType: "user-callback", invoker: "setTimeout" },
                    {
                        duration: 98.4,
                        invokerType: "event-listener",
                        invoker: "WebSocket.onmessage",
                        sourceURL: "http://localhost:5173/assets/index-abc123.js?v=1",
                        sourceFunctionName: "handleMessage",
                        sourceCharPosition: 123456,
                    },
                    { duration: 5, invokerType: "classic-script" },
                    { duration: 3 },
                    { duration: 1 },
                ],
            })
        );
        expect(line).toBe(
            "long-frame 180ms (blocking 130ms): scripts 127ms, render 40ms, style+layout 30ms; " +
                "98ms event-listener WebSocket.onmessage handleMessage @ index-abc123.js:123456; " +
                "20ms user-callback setTimeout; 5ms classic-script; +2 more"
        );
    });

    it("says so when no script ran in the frame (a long render)", () => {
        expect(describeLongFrame(frame({ renderStart: 1010, styleAndLayoutStart: 1020 }))).toBe(
            "long-frame 180ms: scripts 0ms, render 170ms, style+layout 160ms; no script"
        );
    });

    it("shows how much of a script was forced layout", () => {
        const line = describeLongFrame(frame({ scripts: [{ duration: 60, invoker: "rAF", forcedStyleAndLayoutDuration: 44 }] }));
        expect(line).toContain("60ms rAF (forced layout 44ms)");
    });
});

describe("createLongFrameLogger", () => {
    it("logs a few frames a second and counts the rest into the next line", () => {
        let t = 0;
        const lines: string[] = [];
        const log = createLongFrameLogger((l) => lines.push(l), () => t);
        for (let i = 0; i < 9; i++) log(frame());
        expect(lines).toHaveLength(5);
        expect(lines.every((l) => l.startsWith("[perf] long-frame 180ms"))).toBe(true);
        t = 1000;
        log(frame());
        expect(lines).toHaveLength(6);
        expect(lines[5]).toContain("(+4 long frames not logged)");
        t = 1100;
        log(frame());
        expect(lines[6]).not.toContain("not logged");
    });
});

describe("startLongFrameObserver", () => {
    it("does nothing in a runtime without Long Animation Frames", () => {
        vi.stubGlobal("PerformanceObserver", Object.assign(vi.fn(), { supportedEntryTypes: ["longtask"] }));
        expect(startLongFrameObserver()).toBeNull();
    });

    it("logs each frame the runtime reports and stops when asked", () => {
        let deliver: (entries: unknown[]) => void = () => {};
        const disconnect = vi.fn();
        const observe = vi.fn();
        class FakeObserver {
            static supportedEntryTypes = ["long-animation-frame"];
            constructor(cb: (list: { getEntries: () => unknown[] }) => void) {
                deliver = (entries) => cb({ getEntries: () => entries });
            }
            observe = observe;
            disconnect = disconnect;
        }
        vi.stubGlobal("PerformanceObserver", FakeObserver);
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const stop = startLongFrameObserver();
        expect(observe).toHaveBeenCalledWith({ type: "long-animation-frame", buffered: true });
        deliver([frame({ scripts: [{ duration: 90, invoker: "WebSocket.onmessage" }] })]);
        expect(warn).toHaveBeenCalledWith(expect.stringContaining("[perf] long-frame 180ms"));
        stop?.();
        expect(disconnect).toHaveBeenCalled();
        warn.mockRestore();
    });
});
