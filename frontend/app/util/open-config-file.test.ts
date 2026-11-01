// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for open-config-file.ts: a config file opens in the Editor (reusing the tab's, tree collapsed), not the OS editor.

import { beforeEach, describe, expect, it, vi } from "vitest";

const WIN_SETTINGS = String.raw`C:\Users\a\config\settings.json`;
const WIN_WIDGETS = String.raw`C:\Users\a\config\widgets.json`;

const api = vi.hoisted(() => ({
    ensureSettingsFile: vi.fn(async () => String.raw`C:\Users\a\config\settings.json`),
    openSettingsFileInEditor: vi.fn(),
}));
const rpc = vi.hoisted(() => ({
    ReadEditorFileCommand: vi.fn(async () => ({ content: "{}" })),
    WriteEditorFileCommand: vi.fn(async () => undefined),
    PaneOpenCommand: vi.fn(async () => ({ block_id: "b1" })),
}));
const focus = vi.hoisted(() => ({ id: "settings-block" as string | undefined }));

vi.mock("@/app/store/app-api", () => ({ getApi: () => api }));
vi.mock("@/app/store/block-component-registry", () => ({ getFocusedBlockId: () => focus.id }));
vi.mock("@/app/store/global", () => ({ atoms: { activeTabId: () => "tab-1" } }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { ensureConfigFile, openConfigFileInEditor, siblingOf } from "./open-config-file";

beforeEach(() => {
    vi.clearAllMocks();
    focus.id = "settings-block";
    rpc.ReadEditorFileCommand.mockResolvedValue({ content: "{}" });
});

describe("siblingOf", () => {
    it("keeps the folder and its separator", () => {
        expect(siblingOf(WIN_SETTINGS, "widgets.json")).toBe(WIN_WIDGETS);
        expect(siblingOf("/home/a/.config/settings.json", "widgets.json")).toBe("/home/a/.config/widgets.json");
        expect(siblingOf("settings.json", "widgets.json")).toBe("widgets.json");
    });
});

describe("ensureConfigFile", () => {
    it("settings.json is the host's ensured path, and nothing is created beside it", async () => {
        expect(await ensureConfigFile("settings.json")).toBe(WIN_SETTINGS);
        expect(rpc.WriteEditorFileCommand).not.toHaveBeenCalled();
    });

    it("widgets.json beside it is left alone when it exists", async () => {
        expect(await ensureConfigFile("widgets.json")).toBe(WIN_WIDGETS);
        expect(rpc.WriteEditorFileCommand).not.toHaveBeenCalled();
    });

    it("widgets.json is created empty when it does not exist", async () => {
        rpc.ReadEditorFileCommand.mockRejectedValue(new Error("not found"));
        await ensureConfigFile("widgets.json");
        expect(rpc.WriteEditorFileCommand).toHaveBeenCalledWith({}, { path: WIN_WIDGETS, content: "{}\n" });
    });
});

describe("openConfigFileInEditor", () => {
    it("opens the file in the Editor beside the pane, reusing the tab's, and not in the OS editor", async () => {
        await openConfigFileInEditor("settings.json");
        expect(api.openSettingsFileInEditor).not.toHaveBeenCalled();
        expect(rpc.PaneOpenCommand).toHaveBeenCalledWith(
            {},
            {
                view: "editor",
                file: WIN_SETTINGS,
                reuse_editor_pane: true,
                split_reference_block_id: "settings-block",
                split_direction: "right",
            },
            {}
        );
    });

    it("leaves the tree state unset, which srv needs to reuse a pane (a new one opens collapsed)", async () => {
        await openConfigFileInEditor("settings.json");
        const [, data] = rpc.PaneOpenCommand.mock.calls[0] as unknown as [unknown, Record<string, unknown>];
        expect(data).not.toHaveProperty("tree_expanded");
    });

    it("names the tab when no pane has the focus", async () => {
        focus.id = undefined;
        await openConfigFileInEditor("widgets.json");
        const [, data] = rpc.PaneOpenCommand.mock.calls[0] as unknown as [unknown, Record<string, unknown>];
        expect(data).toMatchObject({ view: "editor", reuse_editor_pane: true, origin_tab_id: "tab-1" });
        expect(data).not.toHaveProperty("split_reference_block_id");
    });

    it("opens beside the pane it is given, not the focused one", async () => {
        await openConfigFileInEditor("settings.json", { fromBlockId: "other" });
        const [, data] = rpc.PaneOpenCommand.mock.calls[0] as unknown as [unknown, Record<string, unknown>];
        expect(data).toMatchObject({ split_reference_block_id: "other" });
    });

    it("rejects when the Editor can't open it, so the caller can say so", async () => {
        rpc.PaneOpenCommand.mockRejectedValueOnce(new Error("no tab"));
        await expect(openConfigFileInEditor("settings.json")).rejects.toThrow("no tab");
    });
});
