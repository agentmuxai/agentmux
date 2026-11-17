// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The keys the macOS menu bar shows: the shortcut table's answer for each command, in the form AppKit takes.

import { beforeEach, describe, expect, it } from "vitest";

import { menuKeysOf, type MenuKey } from "./menu-keys";
import { effectiveRows, setUserKeybindings } from "./registry";

const keysNow = (): Map<string, MenuKey> => new Map(menuKeysOf(effectiveRows("mac"), "mac").map((k) => [k.command, k]));

beforeEach(() => setUserKeybindings([], () => false));

describe("menuKeysOf", () => {
    it("gives each command the key the table gives it, as a character and modifiers", () => {
        const keys = keysNow();
        expect(keys.get("tab:new")).toEqual({ command: "tab:new", key: "t", ctrl: false, shift: false, alt: false, meta: true });
        expect(keys.get("tab:close")).toMatchObject({ key: "w", shift: true, meta: true });
        expect(keys.get("app:settings")).toMatchObject({ key: ",", meta: true });
        expect(keys.get("view:zoom:in")).toMatchObject({ key: "=", meta: true });
        expect(keys.get("view:zoom:reset")).toMatchObject({ key: "0", meta: true });
    });

    it("takes a command's first key: the palette's first, the tab chords' bracket", () => {
        const keys = keysNow();
        expect(keys.get("view:command-palette")).toMatchObject({ key: "p", shift: true, meta: true });
        expect(keys.get("tab:next")).toMatchObject({ key: "]", meta: true, shift: false });
    });

    it("puts the person's own key before the default, and drops one they unbound", () => {
        setUserKeybindings(
            [
                { command: "tab:new", key: "meta+y", platform: "mac" },
                { command: "-app:settings", key: "meta+,", platform: "mac" },
            ],
            () => false
        );
        const keys = keysNow();
        expect(keys.get("tab:new")).toMatchObject({ key: "y", meta: true });
        expect(keys.has("app:settings")).toBe(false);
    });

    it("leaves out what a menu item can't carry or must not take from the page", () => {
        const keys = keysNow();
        // A key with no modifier (Escape), and a named key (ctrl+Tab is next tab's later row, not its first).
        expect(keys.has("app:escape")).toBe(false);
        for (const k of keys.values()) {
            expect(k.key).toHaveLength(1);
            expect(k.ctrl || k.alt || k.meta).toBe(true);
        }
        // A row with a context (`when`) or for one pane would run from the menu where the page's handler would not.
        for (const { row } of effectiveRows("mac")) {
            if (row.when || row.pane) expect(menuKeysOf([{ row, source: "meta+q" }], "mac"), row.command).toEqual([]);
        }
    });
});
