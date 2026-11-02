// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { formatBytes } from "./format-bytes";

const KB = 1024;
const MB = KB * 1024;
const GB = MB * 1024;
const TB = GB * 1024;

describe("formatBytes", () => {
    it.each([
        [0, "0 B"],
        [1, "1 B"],
        [812, "812 B"],
        [1023, "1023 B"],
        [1024, "1.0 KB"],
        [1536, "1.5 KB"],
        [4915, "4.8 KB"],
        [9.96 * KB, "10 KB"],
        [10 * KB, "10 KB"],
        [812 * KB, "812 KB"],
        [1023 * KB, "1023 KB"],
        [MB, "1.0 MB"],
        [4_404_019, "4.2 MB"],
        [90.4 * MB, "90.4 MB"],
        [312 * MB, "312.0 MB"],
        [GB, "1.0 GB"],
        [1.5 * GB, "1.5 GB"],
        [2.5 * GB, "2.5 GB"],
        [TB, "1.0 TB"],
        [3.2 * TB, "3.2 TB"],
        [2048 * TB, "2048.0 TB"],
    ])("%d bytes is %s", (bytes, text) => {
        expect(formatBytes(bytes)).toBe(text);
    });

    it("steps up a unit when rounding reaches 1024", () => {
        expect(formatBytes(1023.9 * KB)).toBe("1.0 MB");
        expect(formatBytes(1023.97 * MB)).toBe("1.0 GB");
    });

    it("writes one size one way: 2.5 GB is never 2.3 GB or 2384.2 MB", () => {
        expect(formatBytes(2.5 * GB)).toBe("2.5 GB");
        expect(formatBytes(2.5 * GB)).not.toContain("2560");
        expect(formatBytes(2.5 * GB)).not.toContain("MB");
    });

    it("reads a value that is not a size as 0 B", () => {
        expect(formatBytes(-5)).toBe("0 B");
        expect(formatBytes(Number.NaN)).toBe("0 B");
        expect(formatBytes(Number.POSITIVE_INFINITY)).toBe("0 B");
    });
});

describe("one formatter", () => {
    // Seven copies of this function once printed the same size three ways. A new one fails here.
    it("is the only function in the frontend that turns a byte count into KB, MB or GB", () => {
        const root = path.resolve(__dirname, "..");
        const copies: string[] = [];
        const walk = (dir: string) => {
            for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
                const full = path.join(dir, entry.name);
                if (entry.isDirectory()) {
                    if (entry.name !== "node_modules" && entry.name !== "types") walk(full);
                } else if (/\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)) {
                    const text = fs.readFileSync(full, "utf8");
                    const names = /(?:function|const)\s+(?:formatBytes|formatSize|formatFileSize|humanSize|humanBytes|prettyBytes|fmtBytes|formatFileBytes)\b/;
                    if (names.test(text) && path.relative(root, full).split(path.sep).join("/") !== "util/format-bytes.ts") copies.push(full);
                }
            }
        };
        walk(root);
        expect(copies).toEqual([]);
    });
});
