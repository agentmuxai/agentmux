// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// ☰ → Layouts → "Save layout…" (SPEC_LAYOUT_FILES_2026_09_25.md §6.1–6.2).

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    showSaveLayoutDialog: vi.fn<(name: string) => Promise<string | null>>(),
    SaveLayoutCommand: vi.fn(),
    pushNotification: vi.fn(),
}));

vi.mock("@/store/global", () => ({
    getApi: () => ({ showSaveLayoutDialog: h.showSaveLayoutDialog }),
    pushNotification: h.pushNotification,
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SaveLayoutCommand: h.SaveLayoutCommand } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: { tag: "tab-client" } }));
vi.mock("@/app/store/window-identity", () => ({ windowId: () => "win-7" }));

import { layoutNameFromPath, saveCurrentLayout } from "./save-layout";

beforeEach(() => {
    h.showSaveLayoutDialog.mockReset();
    h.SaveLayoutCommand.mockReset();
    h.pushNotification.mockReset();
});

describe("layoutNameFromPath", () => {
    it("is the file name without the layout extension, on either separator", () => {
        expect(layoutNameFromPath("C:\\Users\\a\\Review setup.agentmux-layout.json")).toBe("Review setup");
        expect(layoutNameFromPath("/home/a/layouts/dev.AgentMux-Layout.JSON")).toBe("dev");
        expect(layoutNameFromPath("/home/a/.agentmux-layout.json")).toBe("Layout");
    });
});

describe("saveCurrentLayout", () => {
    it("does nothing when the dialog is cancelled", async () => {
        h.showSaveLayoutDialog.mockResolvedValue(null);
        await saveCurrentLayout();
        expect(h.SaveLayoutCommand).not.toHaveBeenCalled();
        expect(h.pushNotification).not.toHaveBeenCalled();
    });

    it("saves this window to the chosen file, named after it", async () => {
        h.showSaveLayoutDialog.mockResolvedValue("/home/a/layouts/Review.agentmux-layout.json");
        h.SaveLayoutCommand.mockResolvedValue({ path: "/home/a/layouts/Review.agentmux-layout.json", warnings: [] });
        await saveCurrentLayout();
        expect(h.SaveLayoutCommand).toHaveBeenCalledWith(
            { tag: "tab-client" },
            { window_id: "win-7", name: "Review", path: "/home/a/layouts/Review.agentmux-layout.json" },
        );
        expect(h.pushNotification).toHaveBeenCalledWith(expect.objectContaining({ type: "info", title: "Layout saved" }));
    });

    it("surfaces warnings instead of a plain success", async () => {
        h.showSaveLayoutDialog.mockResolvedValue("/x/a.agentmux-layout.json");
        h.SaveLayoutCommand.mockResolvedValue({ path: "/x/a.agentmux-layout.json", warnings: ["a command looks sensitive"] });
        await saveCurrentLayout();
        const notif = h.pushNotification.mock.calls[0][0];
        expect(notif.type).toBe("warning");
        expect(notif.message).toContain("a command looks sensitive");
    });

    it("shows a failure rather than swallowing it", async () => {
        h.showSaveLayoutDialog.mockResolvedValue("/x/a.agentmux-layout.json");
        h.SaveLayoutCommand.mockRejectedValue(new Error("disk full"));
        await saveCurrentLayout();
        expect(h.pushNotification).toHaveBeenCalledWith(
            expect.objectContaining({ type: "error", message: "disk full" }),
        );
    });
});
