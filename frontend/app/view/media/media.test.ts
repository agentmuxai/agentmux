// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { dirnameOf, extOf } from "./media";

describe("extOf", () => {
    it("returns the lowercase extension without a dot", () => {
        expect(extOf("clips/shot-06.WEBM")).toBe("webm");
        expect(extOf("/a/b/c.png")).toBe("png");
    });

    it("returns empty string when there's no extension", () => {
        expect(extOf("clips/README")).toBe("");
        expect(extOf("")).toBe("");
    });

    it("uses the last dot for a multi-dot filename", () => {
        expect(extOf("shot.v2.final.mp4")).toBe("mp4");
    });
});

describe("dirnameOf", () => {
    it("strips the last posix segment", () => {
        expect(dirnameOf("/home/user/clips/shot.webm")).toBe("/home/user/clips");
    });

    it("strips the last windows segment", () => {
        expect(dirnameOf("C:\\Users\\asafe\\clips\\shot.webm")).toBe("C:\\Users\\asafe\\clips");
    });

    it("returns empty string when there's no separator", () => {
        expect(dirnameOf("shot.webm")).toBe("");
    });
});
