// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// host-keys.json is what crates/cef forwards out of a browser pane. It must
// match the shortcut table; regenerate with UPDATE_HOST_KEYS=1.

import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { hostKeys, hostKeysOf } from "./host-keys";
import { compileRows } from "./registry";

const FILE = join(__dirname, "host-keys.json");

describe("host-keys.json", () => {
    it("matches the shortcut table", () => {
        const generated = JSON.stringify({ mac: hostKeys("mac"), other: hostKeys("other"), linux: hostKeys("linux") }, null, 2) + "\n";
        if (process.env.UPDATE_HOST_KEYS) writeFileSync(FILE, generated);
        expect(readFileSync(FILE, "utf8").replace(/\r\n/g, "\n")).toBe(generated);
    });

    it("forwards window, tab and pane keys but not typing-scoped, find or zoom keys", () => {
        const other = hostKeys("other").map((k) => k.command);
        expect(other).toContain("tab:new");
        expect(other).toContain("pane:focus:next");
        expect(other).toContain("view:command-palette");
        expect(other).not.toContain("pane:focus:left"); // !textInputFocus: the page may be typing
        expect(other).not.toContain("view:zoom:in");
        expect(other).not.toContain("pane:find");
        expect(other).not.toContain("term:paste");
        expect(other).not.toContain("dev:perfHud");
    });

    describe("for the person's own keybindings", () => {
        const keysOf = (entries: unknown[]) => hostKeysOf(compileRows("other", entries as never), "other").map((k) => `${k.command} ${k.source}`);

        it("forwards a key they bind and not one they unbind", () => {
            const keys = keysOf([{ command: "-tab:new", key: "ctrl+shift+t" }, { command: "tab:new", key: "ctrl+shift+y" }]);
            expect(keys).toContain("tab:new ctrl+shift+y");
            expect(keys).not.toContain("tab:new ctrl+shift+t");
        });

        it("is the default list when they have no entries", () => {
            expect(hostKeysOf(compileRows("other", []), "other")).toEqual(hostKeys("other"));
        });

        it("does not forward a key the page may be typing: a bare or shifted letter, a bare Enter or arrow", () => {
            const keys = keysOf(["a", "shift+a", "Enter", "ArrowUp", "shift+ArrowLeft"].map((key) => ({ command: "tab:new", key })));
            expect(keys.filter((k) => k.startsWith("tab:new") && !k.endsWith("ctrl+shift+t"))).toEqual([]);
        });

        it("forwards a function key or a key with Ctrl or Alt", () => {
            const keys = keysOf(["F9", "ctrl+alt+9", "alt+ArrowUp"].map((key) => ({ command: "tab:new", key })));
            expect(keys).toEqual(expect.arrayContaining(["tab:new F9", "tab:new ctrl+alt+9", "tab:new alt+ArrowUp"]));
        });

        it("does not forward a key limited to where the person types, or a chord", () => {
            const keys = keysOf([{ command: "tab:new", key: "ctrl+alt+u", when: "textInputFocus" }, { command: "tab:new", key: "ctrl+k ctrl+u" }]);
            expect(keys.join()).not.toContain("ctrl+alt+u");
            expect(keys.join()).not.toContain("ctrl+k");
        });
    });

    it("uses Windows virtual-key codes", () => {
        const newTab = hostKeys("other").find((k) => k.command === "tab:new");
        expect(newTab).toEqual({ command: "tab:new", source: "ctrl+shift+t", ctrl: true, shift: true, alt: false, meta: false, vk: 0x54 });
        const f6 = hostKeys("mac").find((k) => k.command === "pane:focus:next");
        expect(f6?.vk).toBe(0x75);
    });
});
