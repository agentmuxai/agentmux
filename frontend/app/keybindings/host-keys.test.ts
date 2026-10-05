// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// host-keys.json is what crates/cef forwards out of a browser pane. It must
// match the shortcut table; regenerate with UPDATE_HOST_KEYS=1.

import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { hostKeys } from "./host-keys";

const FILE = join(__dirname, "host-keys.json");

describe("host-keys.json", () => {
    it("matches the shortcut table", () => {
        const generated = JSON.stringify({ mac: hostKeys("mac"), other: hostKeys("other") }, null, 2) + "\n";
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

    it("uses Windows virtual-key codes", () => {
        const newTab = hostKeys("other").find((k) => k.command === "tab:new");
        expect(newTab).toEqual({ command: "tab:new", source: "ctrl+shift+t", ctrl: true, shift: true, alt: false, meta: false, vk: 0x54 });
        const f6 = hostKeys("mac").find((k) => k.command === "pane:focus:next");
        expect(f6?.vk).toBe(0x75);
    });
});
