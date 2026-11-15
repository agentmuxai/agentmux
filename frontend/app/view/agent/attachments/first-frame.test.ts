// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { fitScale, firstFrame, MAX_PDF_BYTES, MAX_VIDEO_BYTES, wantsFrame } from "./first-frame";

describe("wantsFrame", () => {
    it("draws a picture for a PDF or a video of a size worth reading", () => {
        expect(wantsFrame("pdf", 2_000_000)).toBe(true);
        expect(wantsFrame("video", 40_000_000)).toBe(true);
    });

    it("leaves the icon for a file that is too big, empty or of another kind", () => {
        expect(wantsFrame("pdf", MAX_PDF_BYTES + 1)).toBe(false);
        expect(wantsFrame("video", MAX_VIDEO_BYTES + 1)).toBe(false);
        expect(wantsFrame("pdf", 0)).toBe(false);
        expect(wantsFrame("pdf", undefined)).toBe(false);
        expect(wantsFrame("word", 1000)).toBe(false);
        expect(wantsFrame("audio", 1000)).toBe(false);
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
