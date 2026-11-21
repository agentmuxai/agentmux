// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The palette lists every feature a person can start by name (docs/reports/REPORT_COMMAND_PALETTE_AUDIT_2026_11_17.md).
 * Two guards keep it so:
 *  1. every pane view the app registers has an "Open" command, or a reason it has none here;
 *  2. every command the shortcut table binds to a key has a palette entry, or a reason it has none
 *     (`KEY_COMMANDS_WITHOUT_ENTRY`).
 * A new view or a new key row fails here until it has one or the other.
 */
import { describe, expect, it } from "vitest";
import "@/app/block/block-registry";
import { DEFAULT_KEYBINDINGS } from "@/app/keybindings/defaults";
import { listPaneTabs } from "@/app/store/pane-tab-registry";
import { KEY_COMMANDS_WITHOUT_ENTRY } from "@/app/store/palette-key-commands";
import { OPEN_COMMAND_FOR_VIEW } from "@/app/store/palette-open-commands";
import { commandRegistry, registerDefaultCommands } from "./command-registry";

/** Pane views with no "Open" command, each with the reason. */
const VIEWS_WITHOUT_OPEN: Readonly<Record<string, string>> = {
    identity: "the agent settings tab, opened from an agent pane",
    shortcuts: "the empty-pane launcher; a palette split creates one",
};

describe("every pane view can be opened from the palette", () => {
    registerDefaultCommands();
    const views = listPaneTabs()
        .filter((m) => !m.legacyOf)
        .map((m) => m.view);

    it.each(views)("%s has an Open command or a stated reason", (view) => {
        const command = OPEN_COMMAND_FOR_VIEW[view];
        if (command === undefined) {
            expect(VIEWS_WITHOUT_OPEN, `the "${view}" view has no palette command and no reason`).toHaveProperty([view]);
            return;
        }
        expect(commandRegistry.get(command), `"${view}" names ${command}, which is not registered`).toBeDefined();
    });

    it("does not list a reason for a view that has a command, or for one that does not exist", () => {
        for (const view of Object.keys(VIEWS_WITHOUT_OPEN)) {
            expect(OPEN_COMMAND_FOR_VIEW, `${view} has both a command and a reason`).not.toHaveProperty([view]);
            expect(views, `${view} is not a registered view`).toContain(view);
        }
    });
});

describe("every key-bound command can be found in the palette", () => {
    registerDefaultCommands();
    const commands = [...new Set(DEFAULT_KEYBINDINGS.map((row) => row.command))];

    it.each(commands)("%s has an entry or a stated reason", (command) => {
        if (KEY_COMMANDS_WITHOUT_ENTRY[command] !== undefined) {
            expect(commandRegistry.get(command), `${command} has an entry and a reason to have none`).toBeUndefined();
            return;
        }
        expect(commandRegistry.get(command), `${command} is bound to a key but has no palette entry and no reason`).toBeDefined();
    });

    it("does not list a reason for a command the table does not have", () => {
        for (const command of Object.keys(KEY_COMMANDS_WITHOUT_ENTRY)) {
            expect(commands, `${command} is not in the shortcut table`).toContain(command);
        }
    });
});
