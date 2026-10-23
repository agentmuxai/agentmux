// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The palette's Workspace commands (SPEC_WORKSPACES_2026_10_23.md §3.2) run
 * the same actions as ☰ → Workspaces, Delete Workspace… included.
 */

import { describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    newWorkspace: vi.fn(),
    openWorkspacePicker: vi.fn(),
    renameWorkspacePrompt: vi.fn(),
    deleteWorkspacePrompt: vi.fn(),
    exportWorkspace: vi.fn(),
    importWorkspace: vi.fn(),
}));
vi.mock("@/app/window/workspaces", () => h);

import { commandRegistry, registerDefaultCommands } from "./command-registry";

describe("palette commands for workspaces", () => {
    registerDefaultCommands();

    it.each([
        ["workspace:new", "New Workspace…", "newWorkspace"],
        ["workspace:open", "Open Workspace…", "openWorkspacePicker"],
        ["workspace:rename", "Rename Workspace…", "renameWorkspacePrompt"],
        ["workspace:delete", "Delete Workspace…", "deleteWorkspacePrompt"],
        ["workspace:export", "Export Workspace…", "exportWorkspace"],
        ["workspace:import", "Import Workspace…", "importWorkspace"],
    ] as const)("%s (%s) runs %s", async (id, label, action) => {
        expect(commandRegistry.get(id)?.label).toBe(label);
        expect(commandRegistry.get(id)?.category).toBe("Workspace");
        h[action].mockClear();
        expect(commandRegistry.run(id)).toBe(true);
        await vi.waitFor(() => expect(h[action]).toHaveBeenCalledTimes(1));
    });
});
