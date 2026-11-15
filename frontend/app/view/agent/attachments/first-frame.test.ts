// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// @vitest-environment jsdom

import { describe, expect, it, vi } from "vitest";
import { fitScale, firstFrame, MAX_PDF_BYTES, MAX_VIDEO_BYTES, tooBigForFrame, wantsFrame } from "./first-frame";

describe("wantsFrame", () => {
    it("draws a picture for a PDF or a video of a size worth reading", () => {
        expect(wantsFrame("pdf", 2_000_000)).toBe(true);
        expect(wantsFrame("video", 40_000_000)).toBe(true);
    });

    it("leaves the icon for a file that is too big, empty or of another kind", () => {
        expect(wantsFrame("pdf", MAX_PDF_BYTES + 1)).toBe(false);
        expect(wantsFrame("video", MAX_VIDEO_BYTES + 1)).toBe(false);
        expect(wantsFrame("pdf", 0)).toBe(false);
        expect(wantsFrame("word", 1000)).toBe(false);
        expect(wantsFrame("audio", 1000)).toBe(false);
    });
});

describe("a size that isn't known", () => {
    it("is let through, and the length the file announces decides", () => {
        expect(wantsFrame("pdf", undefined)).toBe(true);
        expect(tooBigForFrame("pdf", String(MAX_PDF_BYTES + 1))).toBe(true);
        expect(tooBigForFrame("video", String(MAX_PDF_BYTES + 1))).toBe(false);
        expect(tooBigForFrame("pdf", "2048")).toBe(false);
        expect(tooBigForFrame("pdf", null)).toBe(false);
        expect(tooBigForFrame("pdf", "nonsense")).toBe(false);
    });
});

describe("fitScale", () => {
    it("shrinks a page or a frame to the tile's size and never enlarges a small one", () => {
        expect(fitScale(612, 792)).toBeCloseTo(192 / 792);
        expect(fitScale(1920, 1080)).toBeCloseTo(192 / 1920);
        expect(fitScale(100, 50)).toBe(1);
        expect(fitScale(0, 0)).toBe(1);
    });
});

describe("firstFrame", () => {
    it("reads nothing from an empty file or one over its limit", async () => {
        expect(await firstFrame("pdf", new Blob([]))).toBeNull();
        expect(await firstFrame("video", { size: MAX_VIDEO_BYTES + 1 } as Blob)).toBeNull();
        expect(await firstFrame("pdf", { size: MAX_PDF_BYTES + 1 } as Blob)).toBeNull();
    });
});

describe("a video that never gives a frame", () => {
    it("is let go of when the time is up: its element stops loading and its file's URL is revoked", async () => {
        vi.useFakeTimers();
        const revoked: string[] = [];
        vi.stubGlobal("URL", Object.assign(URL, { createObjectURL: () => "blob:stalled", revokeObjectURL: (u: string) => revoked.push(u) }));
        const video = document.createElement("video");
        const load = vi.spyOn(video, "load").mockImplementation(() => {});
        vi.spyOn(document, "createElement").mockReturnValue(video);
        const result = firstFrame("video", new Blob(["x"]));
        await vi.advanceTimersByTimeAsync(9000);
        expect(await result).toBeNull();
        expect(revoked).toEqual(["blob:stalled"]);
        expect(load).toHaveBeenCalled();
        expect(video.getAttribute("src")).toBeNull();
        vi.useRealTimers();
        vi.restoreAllMocks();
        vi.unstubAllGlobals();
    });
});
