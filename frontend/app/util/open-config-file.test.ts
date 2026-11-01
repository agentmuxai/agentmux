// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for open-config-file.ts: a config file opens in the Editor (reusing the tab's, tree collapsed), not the OS editor.

import { beforeEach, describe, expect, it, vi } from "vitest";

const WIN_SETTINGS = String.raw`C:\Users\a\config\settings.json`;
const WIN_WIDGETS = String.raw`C:\Users\a\config\widgets.json`;
const WIN_FOLDER = "C:\\Users\\a\\config\\";

const api = vi.hoisted(() => ({
    ensureSettingsFile: vi.fn(async () => "C:\\Users\\a\\config\\settings.json"),
    openSettingsFileInEditor: vi.fn(),
}));
const rpc = vi.hoisted(() => ({
    ListEditorDirCommand: vi.fn(async (_c: unknown, _d: unknown) => ({ path: "", entries: [{ name: "settings.json" }, { name: "widgets.json" }] })),
    WriteEditorFileCommand: vi.fn(async () => undefined),
    PaneOpenCommand: vi.fn(async () => ({ block_id: "b1" })),
}));
const blocks = vi.hoisted(() => ({ focused: "settings-block" as string | undefined, all: [["tab-block", {}]] as [string, unknown][] }));

vi.mock("@/app/store/app-api", () => ({ getApi: () => api }));
vi.mock("@/app/store/block-component-registry", () => ({
    getFocusedBlockId: () => blocks.focused,
    getAllBlockComponentModelEntries: () => blocks.all,
}));
vi.mock("@/app/store/global", () => ({ atoms: { activeTabId: () => "tab-1" } }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { ensureConfigFile, openConfigFileInEditor, siblingOf } from "./open-config-file";

const sent = () => rpc.PaneOpenCommand.mock.calls[0] as unknown as [unknown, Record<string, unknown>];

beforeEach(() => {
    vi.clearAllMocks();
    blocks.focused = "settings-block";
    blocks.all = [["tab-block", {}]];
    rpc.ListEditorDirCommand.mockResolvedValue({ path: "", entries: [{ name: "settings.json" }, { name: "widgets.json" }] });
});

describe("siblingOf", () => {
    it("keeps the folder and its separator", () => {
        expect(siblingOf(WIN_SETTINGS, "widgets.json")).toBe(WIN_WIDGETS);
        expect(siblingOf("/home/a/.config/settings.json", "widgets.json")).toBe("/home/a/.config/widgets.json");
        expect(siblingOf("settings.json", "widgets.json")).toBe("widgets.json");
    });
});

describe("ensureConfigFile", () => {
    it("settings.json is the host's ensured path, and nothing is listed or created", async () => {
        expect(await ensureConfigFile("settings.json")).toBe(WIN_SETTINGS);
        expect(rpc.ListEditorDirCommand).not.toHaveBeenCalled();
        expect(rpc.WriteEditorFileCommand).not.toHaveBeenCalled();
    });

    it("widgets.json beside it is left alone when it is in the folder, whatever the case of its name", async () => {
        expect(await ensureConfigFile("widgets.json")).toBe(WIN_WIDGETS);
        rpc.ListEditorDirCommand.mockResolvedValue({ path: "", entries: [{ name: "Widgets.JSON" }] });
        await ensureConfigFile("widgets.json");
        expect(rpc.ListEditorDirCommand).toHaveBeenCalledWith({}, { path: WIN_FOLDER });
        expect(rpc.WriteEditorFileCommand).not.toHaveBeenCalled();
    });

    it("widgets.json is created empty when the folder does not have it", async () => {
        rpc.ListEditorDirCommand.mockResolvedValue({ path: "", entries: [{ name: "settings.json" }] });
        await ensureConfigFile("widgets.json");
        expect(rpc.WriteEditorFileCommand).toHaveBeenCalledWith({}, { path: WIN_WIDGETS, content: "{}\n" });
    });

    it("never writes when the folder can't be listed, so a file that can't be read right now is not erased", async () => {
        rpc.ListEditorDirCommand.mockRejectedValue(new Error("permission denied"));
        await expect(ensureConfigFile("widgets.json")).rejects.toThrow("permission denied");
        expect(rpc.WriteEditorFileCommand).not.toHaveBeenCalled();
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
        expect(sent()[1]).not.toHaveProperty("tree_expanded");
    });

    it("names a pane of the tab when none has the focus, so srv can still reuse the tab's Editor", async () => {
        blocks.focused = undefined;
        await openConfigFileInEditor("widgets.json");
        expect(sent()[1]).toMatchObject({ view: "editor", reuse_editor_pane: true, split_reference_block_id: "tab-block" });
    });

    it("names the tab when the window has no pane at all", async () => {
        blocks.focused = undefined;
        blocks.all = [];
        await openConfigFileInEditor("widgets.json");
        expect(sent()[1]).toMatchObject({ view: "editor", origin_tab_id: "tab-1" });
        expect(sent()[1]).not.toHaveProperty("split_reference_block_id");
    });

    it("opens beside the pane it is given, not the focused one", async () => {
        await openConfigFileInEditor("settings.json", { fromBlockId: "other" });
        expect(sent()[1]).toMatchObject({ split_reference_block_id: "other" });
    });

    it("rejects when the Editor can't open it, so the caller can say so", async () => {
        rpc.PaneOpenCommand.mockRejectedValueOnce(new Error("no tab"));
        await expect(openConfigFileInEditor("settings.json")).rejects.toThrow("no tab");
    });
});
