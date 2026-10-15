// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { formatFileSize, shareLabel, tokensLabel, tokensTitle } from "./memory-tokens";

describe("memory-tokens", () => {
    it("labels tokens compactly, always as an estimate", () => {
        expect(tokensLabel(0)).toBe("~0 tok");
        expect(tokensLabel(999)).toBe("~999 tok");
        expect(tokensLabel(1234)).toBe("~1.2k tok");
        expect(tokensLabel(18_400)).toBe("~18k tok");
    });

    it("puts the exact estimate, bytes and lines in the tooltip, then how it was estimated", () => {
        expect(tokensTitle(1234, 4915, 86)).toBe("~1,234 tokens (est.) · 4.8 KB · 86 lines\nEstimated: characters ÷ 4");
        expect(tokensTitle(1, 4, 1)).toBe("~1 token (est.) · 4 B · 1 line\nEstimated: characters ÷ 4");
        expect(tokensTitle(1234)).toBe("~1,234 tokens (est.)\nEstimated: characters ÷ 4");
    });

    it("shares round to a whole percent, say <1% for a sliver, and nothing with no total", () => {
        expect(shareLabel(7, 40)).toBe("18%");
        expect(shareLabel(1, 1000)).toBe("<1%");
        expect(shareLabel(5, 0)).toBe("");
        expect(shareLabel(0, 40)).toBe("");
    });

    it("keeps the byte label it took over from MemoryFileCard", () => {
        expect(formatFileSize(512)).toBe("512 B");
        expect(formatFileSize(2048)).toBe("2.0 KB");
        expect(formatFileSize(-1)).toBe("");
    });
});
