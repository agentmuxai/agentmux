// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Every item of the macOS menu bar runs a command by id (crates/cef/src/macos_menu.rs). An id the command registry
 * doesn't have does nothing when clicked, and "Settings…" once ran the settings file's command instead of the
 * Settings pane's: this reads the menu's ids from the source and holds each to a registered command.
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it, vi } from "vitest";

vi.mock("@/util/app-api", () => ({ getApi: () => ({}) }));

import { commandRegistry, registerDefaultCommands } from "./command-registry";

const source = readFileSync(resolve(process.cwd(), "crates/cef/src/macos_menu.rs"), "utf8");
const ids = [...source.matchAll(/add_cmd\(\w+, target, [^,]+, "([^"]+)"/g)].map((m) => m[1]);

describe("the macOS menu bar's commands", () => {
    registerDefaultCommands();

    it("names a command the registry has, for every item", () => {
        expect(ids.length).toBeGreaterThan(10);
        for (const id of ids) expect(commandRegistry.get(id), id).toBeDefined();
    });

    it("opens the Settings pane from Settings…, as the palette's Settings does", () => {
        expect(ids).toContain("app:settings");
        expect(ids).not.toContain("dev:open_settings");
        expect(commandRegistry.get("app:settings")?.label).toBe("Settings");
    });
});
