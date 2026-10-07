// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `searchCommands`: the command palette's search. Typing a command's name lists
// that command first, and not the commands that only look like it ("Close Tab"
// must not list "Close Pane"); a typo still finds it. Runs the real Fuse.js
// fallback, not a mock.

import { describe, expect, it } from "vitest";

import type { CommandEntry } from "@/app/store/command-registry";
import { searchCommands } from "./command-palette-search";

const cmd = (id: string, label: string, category: string, keywords?: string): CommandEntry => ({
    id,
    label,
    category,
    keywords,
    execute: () => {},
});

// Real commands from command-registry.ts, in the order the palette passes them
// in (sortCommands: category order, then label).
const COMMANDS: CommandEntry[] = [
    cmd("open:agent", "Open Agent", "Open"),
    cmd("open:terminal", "Open Terminal", "Open"),
    cmd("split:down", "Split Down", "Split"),
    cmd("split:left", "Split Left", "Split"),
    cmd("split:right", "Split Right", "Split"),
    cmd("window:close", "Close Window", "Window"),
    cmd("window:new", "New Window", "Window"),
    cmd("window:maximize", "Toggle Maximize", "Window"),
    cmd("tab:close", "Close Tab", "Tab"),
    cmd("tab:new", "New Tab", "Tab"),
    cmd("pane:close", "Close Pane", "Pane"),
    cmd("pane:magnify", "Toggle Magnify", "Pane"),
    cmd("dev:open_settings", "Open Settings File", "Dev"),
    cmd("view:zoom:reset", "Actual Size", "View"),
    cmd("app:connectors", "Connectors", "App", "accounts sign in login mcp servers armory"),
    cmd("app:memory", "Memory", "App", "global personal memory skills bundles abf armory knowledge"),
    cmd("view:zoom:in", "Zoom In", "View"),
    cmd("view:zoom:out", "Zoom Out", "View"),
];

const labels = (q: string) => searchCommands(COMMANDS, q).map((c) => c.label);

describe("searchCommands", () => {
    it("returns the commands unchanged, in palette order, for a blank query", () => {
        expect(searchCommands(COMMANDS, "")).toBe(COMMANDS);
        expect(searchCommands(COMMANDS, "  ")).toBe(COMMANDS);
    });

    it("lists an exact command name first", () => {
        expect(labels("Open Agent")[0]).toBe("Open Agent");
        expect(labels("new tab")[0]).toBe("New Tab");
    });

    it("does not list near-misses that do not contain the query", () => {
        expect(labels("Close Tab")).toEqual(["Close Tab"]);
        expect(labels("Toggle Maximize")).toEqual(["Toggle Maximize"]);
        expect(labels("Open Agent")).toEqual(["Open Agent"]);
    });

    it("lists every command whose label contains the query, keeping palette order among equals", () => {
        expect(labels("close")).toEqual(["Close Window", "Close Tab", "Close Pane"]);
    });

    it("ranks label matches above keyword, category and id matches", () => {
        // Actual Size comes before Zoom In in palette order, but only its id
        // (view:zoom:reset) says zoom.
        expect(labels("zoom")).toEqual(["Zoom In", "Zoom Out", "Actual Size"]);
        // Toggle Maximize is in the Window category, its label does not say so.
        expect(labels("window")).toEqual(["Close Window", "New Window", "Toggle Maximize"]);
    });

    it("finds a command by its keywords", () => {
        expect(labels("armory")).toEqual(["Connectors", "Memory"]);
        expect(labels("login")).toEqual(["Connectors"]);
    });

    it("still finds a command through a typo", () => {
        expect(labels("Spilt Right")[0]).toBe("Split Right");
        expect(labels("magnfy")[0]).toBe("Toggle Magnify");
    });

    it("returns nothing when no command matches even loosely", () => {
        expect(labels("xyzzy_no_such_command")).toEqual([]);
    });
});
