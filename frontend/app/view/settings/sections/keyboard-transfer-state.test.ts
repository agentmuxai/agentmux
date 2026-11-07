// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it } from "vitest";

import { setUserKeybindings } from "@/app/keybindings/registry";
import { exportText, mergeImport, parseImport, previewImport, previewText } from "./keyboard-transfer-state";

beforeEach(() => void setUserKeybindings([], () => false));

const CURRENT = [
    { command: "-tab:new", key: "ctrl+shift+t", platform: "other" },
    { command: "tab:new", key: "ctrl+alt+t", platform: "other" },
    { command: "-pane:close", platform: "other" },
];

describe("parseImport", () => {
    it("reads an array, or settings that hold one", () => {
        expect(parseImport('[{"command":"-tab:close"}]')).toEqual({ entries: [{ command: "-tab:close" }] });
        expect(parseImport('{"keybindings":[{"command":"-tab:close"}],"theme":"x"}')).toEqual({ entries: [{ command: "-tab:close" }] });
    });

    it("says why it cannot read the text", () => {
        expect(parseImport("   ")).toEqual({ error: "Paste the exported keybindings." });
        expect(parseImport("[{")).toMatchObject({ error: expect.stringMatching(/^That is not valid JSON: /) });
        expect(parseImport('{"theme":"x"}')).toEqual({ error: 'Expected an array of entries, or settings with a "keybindings" array.' });
        expect(parseImport("3")).toEqual({ error: 'Expected an array of entries, or settings with a "keybindings" array.' });
        expect(parseImport("null")).toEqual({ error: 'Expected an array of entries, or settings with a "keybindings" array.' });
    });

    it("reads back what export wrote", () => {
        expect(parseImport(exportText(CURRENT))).toEqual({ entries: CURRENT });
    });
});

describe("previewImport", () => {
    const incoming = [
        { command: "-tab:new", platform: "other" },
        { command: "tab:new", key: "ctrl+shift+y", platform: "other" },
        { command: "tab:next", key: "ctrl+alt+n", platform: "other" },
        { command: "tab:ner", key: "x" },
    ];

    it("keeps the entries the registry would apply and names the ones it would skip", () => {
        const p = previewImport(CURRENT, incoming);
        expect(p.usable).toEqual(incoming.slice(0, 3));
        expect(p.unusable).toEqual([{ index: 3, message: 'unknown command "tab:ner"' }]);
    });

    it("refuses a key the system takes before the app, as the recorder does, but not an unbind of it", () => {
        const p = previewImport([], [
            { command: "tab:close", key: "meta+q", platform: "mac" },
            { command: "tab:close", key: "alt+F4" },
            { command: "-tab:close", key: "meta+q", platform: "mac" },
            { command: "tab:close", key: "ctrl+alt+t", platform: "other" },
            { command: "tab:close", key: "meta+q", platform: "other" },
        ]);
        expect(p.unusable.map((u) => u.index)).toEqual([0, 1, 4]);
        expect(p.unusable[0].message).toMatch(/^meta\+q cannot be used: AgentMux's own menu takes/);
        expect(p.unusable[1].message).toMatch(/^alt\+F4 cannot be used: Windows takes Alt\+F4/);
        // An unbind, and a key only the desktop often takes (the recorder asks "Use it anyway"), are imported.
        expect(p.usable).toHaveLength(2);
    });

    it("counts the commands whose keys of your own the import replaces", () => {
        // New tab has entries of its own that differ; Next tab has none.
        expect(previewImport(CURRENT, incoming).replaced).toBe(1);
    });

    it("sees what the setting already has, word for word", () => {
        const p = previewImport(CURRENT, CURRENT);
        expect(p.already).toBe(3);
        expect(p.replaced).toBe(0);
    });

    it("says it in a sentence", () => {
        expect(previewText(previewImport(CURRENT, incoming))).toBe("3 entries to import; 1 command with keys of your own that merging replaces; 1 not usable, skipped.");
        expect(previewText(previewImport([], [{ command: "-tab:close", platform: "other" }]))).toBe("1 entry to import.");
        expect(previewText(previewImport(CURRENT, CURRENT))).toBe("3 entries to import; 3 already in your setting.");
    });
});

describe("mergeImport", () => {
    it("takes the commands it gives keys to from the import and leaves the rest", () => {
        const usable = previewImport(CURRENT, [
            { command: "-tab:new", platform: "other" },
            { command: "tab:new", key: "ctrl+shift+y", platform: "other" },
        ]).usable;
        expect(mergeImport(CURRENT, usable)).toEqual([{ command: "-pane:close", platform: "other" }, ...usable]);
    });

    it("keeps the other platform's entries of the same command", () => {
        const mac = { command: "tab:new", key: "meta+alt+t", platform: "mac" };
        const usable = [{ command: "tab:new", key: "ctrl+shift+y", platform: "other" }];
        expect(mergeImport([mac, ...CURRENT], usable)).toEqual([mac, { command: "-pane:close", platform: "other" }, ...usable]);
    });

    it("keeps entries that are not usable as they are", () => {
        const broken = { command: "tab:new", key: "a b c", platform: "other" };
        const usable = [{ command: "tab:next", key: "ctrl+alt+n", platform: "other" }];
        expect(mergeImport([broken], usable)).toEqual([broken, ...usable]);
    });
});
