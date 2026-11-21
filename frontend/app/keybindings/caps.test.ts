// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { formatKey } from "./keys";
import { keyCaps, keyCapsWords } from "./caps";

describe("keyCaps", () => {
    it("gives one cap per key", () => {
        expect(keyCaps("Ctrl+Shift+P")).toEqual([["Ctrl", "Shift", "P"]]);
        expect(keyCaps("F2")).toEqual([["F2"]]);
    });

    it("gives the two steps of a chord as two groups", () => {
        expect(keyCaps("Ctrl+Shift+S then Right")).toEqual([["Ctrl", "Shift", "S"], ["Right"]]);
    });

    it("reads macOS symbols as one cap each, and the key after them as the last", () => {
        expect(keyCaps("⇧⌘P")).toEqual([["⇧", "⌘", "P"]]);
        expect(keyCaps("⌃⌥⇧⌘↑")).toEqual([["⌃", "⌥", "⇧", "⌘", "↑"]]);
        expect(keyCaps("⌘T then F")).toEqual([["⌘", "T"], ["F"]]);
    });

    it("keeps the plus key, whose name is the separator", () => {
        expect(keyCaps("Ctrl++")).toEqual([["Ctrl", "+"]]);
        expect(keyCaps("+")).toEqual([["+"]]);
    });

    it("gives nothing for no shortcut", () => {
        expect(keyCaps("")).toEqual([]);
        expect(keyCaps("  ")).toEqual([]);
    });

    it("reads back what formatKey writes, on every platform", () => {
        for (const platform of ["mac", "linux", "other"] as const) {
            for (const spec of ["ctrl+shift+p", "mod+t", "mod+shift+s right", "f6"]) {
                const label = formatKey(spec, platform);
                const joined = keyCaps(label)
                    .map((caps) => (platform === "mac" ? caps.join("") : caps.join("+")))
                    .join(" then ");
                expect(joined).toBe(label);
            }
        }
    });
});

describe("keyCapsWords", () => {
    it("says the keys in words, a chord's steps joined by then", () => {
        expect(keyCapsWords("Ctrl+Shift+P")).toBe("Ctrl Shift P");
        expect(keyCapsWords("Ctrl+Shift+S then Right")).toBe("Ctrl Shift S then Right");
    });

    it("names the macOS symbols", () => {
        expect(keyCapsWords("⇧⌘P")).toBe("Shift Command P");
        expect(keyCapsWords("⌃⌥T")).toBe("Control Option T");
    });

    it("gives nothing for no shortcut", () => {
        expect(keyCapsWords("")).toBe("");
    });
});
