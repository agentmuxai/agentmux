// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { resumeOffsetAfterTrim } from "./term-resume-offset";

describe("resumeOffsetAfterTrim", () => {
    it("leaves the usual bookkeeping alone when nothing was dropped", () => {
        expect(resumeOffsetAfterTrim(100, 100, 50)).toBeNull();
        expect(resumeOffsetAfterTrim(0, 0, 50)).toBeNull();
    });

    it("leaves it alone when srv doesn't say where the body starts", () => {
        expect(resumeOffsetAfterTrim(100, undefined, 50)).toBeNull();
    });

    it("is the absolute end of the body when the head was dropped", () => {
        // Asked from 100, the file now starts at 5000 and 700 bytes came back.
        expect(resumeOffsetAfterTrim(100, 5000, 700)).toBe(5700);
        expect(resumeOffsetAfterTrim(0, 1, 9)).toBe(10);
    });
});
