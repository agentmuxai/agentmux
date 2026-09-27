// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { formatBytes } from "./format-bytes";

describe("formatBytes", () => {
    it("uses whole numbers below a megabyte", () => {
        expect(formatBytes(0)).toBe("0 B");
        expect(formatBytes(812)).toBe("812 B");
        expect(formatBytes(812 * 1024)).toBe("812 KB");
    });

    it("uses one decimal from a megabyte up", () => {
        expect(formatBytes(1.8 * 1024 * 1024)).toBe("1.8 MB");
        expect(formatBytes(1024 * 1024 * 1024)).toBe("1.0 GB");
        expect(formatBytes(214.3 * 1024 * 1024)).toBe("214.3 MB");
    });

    it("treats nonsense as zero", () => {
        expect(formatBytes(-5)).toBe("0 B");
        expect(formatBytes(Number.NaN)).toBe("0 B");
    });
});
