// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The developer commands follow dev mode; the toggle itself and the recovery
 * commands never do. docs/specs/SPEC_DEV_MODE_SETTING_2026_10_28.md §4.4, §6.
 */
import { describe, expect, it, vi } from "vitest";

const devMode = vi.hoisted(() => ({ on: false, toggled: 0 }));
vi.mock("@/app/store/dev-mode", () => ({
    devModeOn: () => devMode.on,
    toggleDevMode: () => {
        devMode.toggled++;
    },
}));

import { commandRegistry, isCommandAvailable, registerDefaultCommands } from "./command-registry";

describe("dev mode and the command palette", () => {
    registerDefaultCommands();
    const listed = (id: string) => {
        const entry = commandRegistry.get(id);
        return entry != null && isCommandAvailable(entry);
    };

    it("Toggle DevTools is unavailable, and declined by run, while dev mode is off", () => {
        devMode.on = false;
        expect(listed("dev:devtools")).toBe(false);
        expect(commandRegistry.run("dev:devtools")).toBe(false);
    });

    it("Toggle DevTools is available while dev mode is on", () => {
        devMode.on = true;
        expect(listed("dev:devtools")).toBe(true);
    });

    it("Toggle Dev Mode is always there, and runs", async () => {
        for (const on of [false, true]) {
            devMode.on = on;
            devMode.toggled = 0;
            expect(listed("view:devmode:toggle")).toBe(true);
            expect(commandRegistry.run("view:devmode:toggle")).toBe(true);
            await Promise.resolve();
            expect(devMode.toggled).toBe(1);
        }
    });

    it("Restart Backend and Open Settings File stay available with dev mode off, in the Window category", () => {
        devMode.on = false;
        for (const id of ["dev:restart_backend", "dev:open_settings"]) {
            expect(listed(id)).toBe(true);
            expect(commandRegistry.get(id)?.category).toBe("Window");
        }
    });
});
