// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The palette's config-file commands: the Editor is the default way in, the system editor a secondary one.
 */
import { describe, expect, it, vi } from "vitest";

const opened = vi.hoisted(() => ({ files: [] as string[], system: 0 }));
vi.mock("@/app/util/open-config-file", () => ({
    openConfigFileOrTell: async (file: string) => {
        opened.files.push(file);
    },
}));
vi.mock("@/app/store/app-api", () => ({
    getApi: () => ({
        openSettingsFileInEditor: async () => {
            opened.system++;
        },
    }),
}));

import { commandRegistry, registerDefaultCommands } from "./command-registry";

describe("config files in the command palette", () => {
    registerDefaultCommands();
    const run = async (id: string) => {
        expect(commandRegistry.run(id)).toBe(true);
        await new Promise((r) => setTimeout(r, 0));
    };

    it("Open Settings File opens settings.json in the Editor, not the system editor", async () => {
        opened.files = [];
        opened.system = 0;
        await run("dev:open_settings");
        expect(opened.files).toEqual(["settings.json"]);
        expect(opened.system).toBe(0);
    });

    it("Open Widgets File opens widgets.json in the Editor", async () => {
        opened.files = [];
        await run("dev:open_widgets_file");
        expect(opened.files).toEqual(["widgets.json"]);
    });

    it("Open Settings File in System Editor is the secondary route, and the only one that uses the host", async () => {
        opened.files = [];
        opened.system = 0;
        await run("dev:open_settings_system");
        expect(opened.system).toBe(1);
        expect(opened.files).toEqual([]);
    });
});
