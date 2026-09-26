// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// ☰ → Layouts → "Open layout…" (SPEC_LAYOUT_FILES_2026_09_25.md §3.5).

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LayoutPreviewResult } from "@/types/rpc/LayoutPreviewResult";

const h = vi.hoisted(() => ({
    showOpenLayoutDialog: vi.fn<() => Promise<string | null>>(),
    openWorkspaceInNewWindow: vi.fn<(workspaceId: string) => Promise<string>>(),
    PreviewLayoutCommand: vi.fn(),
    OpenLayoutCommand: vi.fn(),
    DeleteWorkspace: vi.fn(),
    pushNotification: vi.fn(),
    openModal: vi.fn(),
}));

vi.mock("@/store/global", () => ({
    getApi: () => ({
        showOpenLayoutDialog: h.showOpenLayoutDialog,
        openWorkspaceInNewWindow: h.openWorkspaceInNewWindow,
    }),
    pushNotification: h.pushNotification,
}));
vi.mock("@/app/store/modalmodel", () => ({ openModal: h.openModal }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { PreviewLayoutCommand: h.PreviewLayoutCommand, OpenLayoutCommand: h.OpenLayoutCommand },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: { tag: "tab-client" } }));
vi.mock("@/app/store/services", () => ({ WorkspaceService: { DeleteWorkspace: h.DeleteWorkspace } }));
vi.mock("@/app/store/window-identity", () => ({ windowId: () => "win-7" }));

import { LayoutPreviewModal } from "./layout-preview-modal";
import { applyLayout, openLayoutFromFile } from "./open-layout";

const PREVIEW: LayoutPreviewResult = {
    name: "Work",
    trusted: false,
    tabs: [{ name: "Work", panes: ["Agent: Rev", "Terminal (command not run: `npm test`)"] }],
    commands: ["npm test"],
    notes: ["There's no agent “Ghost” here — its pane opens the agent picker."],
};

beforeEach(() => {
    for (const fn of Object.values(h)) fn.mockReset();
});
afterEach(cleanup);

describe("openLayoutFromFile", () => {
    it("does nothing when the dialog is cancelled", async () => {
        h.showOpenLayoutDialog.mockResolvedValue(null);
        await openLayoutFromFile();
        expect(h.PreviewLayoutCommand).not.toHaveBeenCalled();
        expect(h.openModal).not.toHaveBeenCalled();
    });

    it("previews the chosen file before opening anything", async () => {
        h.showOpenLayoutDialog.mockResolvedValue("/l/work.agentmux-layout.json");
        h.PreviewLayoutCommand.mockResolvedValue(PREVIEW);
        await openLayoutFromFile();
        expect(h.PreviewLayoutCommand).toHaveBeenCalledWith({ tag: "tab-client" }, { path: "/l/work.agentmux-layout.json" });
        expect(h.OpenLayoutCommand).not.toHaveBeenCalled();
        const [component, props] = h.openModal.mock.calls[0];
        expect(component).toBe(LayoutPreviewModal);
        expect(props.preview).toEqual(PREVIEW);

        h.OpenLayoutCommand.mockResolvedValue({ workspace_id: "ws-mine", tab_ids: ["t1"], notes: [] });
        await props.onOpen({ runCommands: true, newWindow: false });
        expect(h.OpenLayoutCommand).toHaveBeenCalledWith(
            { tag: "tab-client" },
            { path: "/l/work.agentmux-layout.json", window_id: "win-7", run_commands: true, new_window: false },
        );
    });

    it("reports a file it can't read instead of swallowing it", async () => {
        h.showOpenLayoutDialog.mockResolvedValue("/l/bad.agentmux-layout.json");
        h.PreviewLayoutCommand.mockRejectedValue(new Error("saved by a newer AgentMux"));
        await openLayoutFromFile();
        expect(h.openModal).not.toHaveBeenCalled();
        expect(h.pushNotification).toHaveBeenCalledWith(
            expect.objectContaining({ type: "error", message: "saved by a newer AgentMux" }),
        );
    });
});

describe("applyLayout", () => {
    it("adding to this window opens no new window", async () => {
        h.OpenLayoutCommand.mockResolvedValue({ workspace_id: "ws-mine", tab_ids: ["t1"], notes: [] });
        await applyLayout("/l/x.agentmux-layout.json", { runCommands: false, newWindow: false });
        expect(h.openWorkspaceInNewWindow).not.toHaveBeenCalled();
        const notif = h.pushNotification.mock.calls[0][0];
        expect(notif.type).toBe("info");
        expect(notif.message).toBe("1 tab added");
    });

    it("opens a new window onto the workspace the layout was built in", async () => {
        h.OpenLayoutCommand.mockResolvedValue({ workspace_id: "ws-new", tab_ids: ["t1", "t2"], notes: [] });
        h.openWorkspaceInNewWindow.mockResolvedValue("win-label");
        await applyLayout("/l/x.agentmux-layout.json", { runCommands: true, newWindow: true });
        expect(h.OpenLayoutCommand).toHaveBeenCalledWith(
            { tag: "tab-client" },
            { path: "/l/x.agentmux-layout.json", window_id: "win-7", run_commands: true, new_window: true },
        );
        expect(h.openWorkspaceInNewWindow).toHaveBeenCalledWith("ws-new");
        expect(h.DeleteWorkspace).not.toHaveBeenCalled();
        const notif = h.pushNotification.mock.calls[0][0];
        expect(notif.type).toBe("info");
        expect(notif.title).toBe("Layout opened in a new window");
        expect(notif.message).toBe("2 tabs");
    });

    it("removes the layout again when its window doesn't open", async () => {
        h.OpenLayoutCommand.mockResolvedValue({ workspace_id: "ws-new", tab_ids: ["t1"], notes: [] });
        h.openWorkspaceInNewWindow.mockRejectedValue(new Error("a pane is currently closing; retry shortly"));
        h.DeleteWorkspace.mockResolvedValue("");
        await applyLayout("/l/x.agentmux-layout.json", { runCommands: false, newWindow: true });
        expect(h.DeleteWorkspace).toHaveBeenCalledWith("ws-new");
        const notif = h.pushNotification.mock.calls[0][0];
        expect(notif.type).toBe("error");
        expect(notif.message).toContain("retry shortly");
    });

    it("says what didn't come across as a warning", async () => {
        h.OpenLayoutCommand.mockResolvedValue({
            workspace_id: "ws-mine",
            tab_ids: ["t1", "t2"],
            notes: ["An agent didn't start"],
        });
        await applyLayout("/l/x.agentmux-layout.json", { runCommands: false, newWindow: false });
        const notif = h.pushNotification.mock.calls[0][0];
        expect(notif.type).toBe("warning");
        expect(notif.title).toContain("2 tabs added");
        expect(notif.message).toContain("An agent didn't start");
    });

    it("reports a failed open, and opens no window for it", async () => {
        h.OpenLayoutCommand.mockRejectedValue(new Error("none of the layout's tabs could be created"));
        await applyLayout("/l/x.agentmux-layout.json", { runCommands: false, newWindow: true });
        expect(h.openWorkspaceInNewWindow).not.toHaveBeenCalled();
        expect(h.pushNotification).toHaveBeenCalledWith(expect.objectContaining({ type: "error" }));
    });
});

describe("LayoutPreviewModal", () => {
    it("lists tabs, notes and commands, and holds an untrusted file's commands by default", async () => {
        const onOpen = vi.fn(() => Promise.resolve());
        const close = vi.fn();
        render(() => <LayoutPreviewModal preview={PREVIEW} onOpen={onOpen} close={close} />);
        expect(screen.getByText("Agent: Rev")).toBeTruthy();
        expect(screen.getByTestId("layout-preview-notes").textContent).toContain("Ghost");
        expect(screen.getByTestId("layout-preview-commands").textContent).toContain("wasn't saved by this AgentMux");
        const box = screen.getByTestId("layout-preview-run-commands") as HTMLInputElement;
        expect(box.checked).toBe(false);

        fireEvent.click(box);
        fireEvent.click(screen.getByText("Open"));
        await vi.waitFor(() => expect(onOpen).toHaveBeenCalledWith({ runCommands: true, newWindow: true }));
        await vi.waitFor(() => expect(close).toHaveBeenCalled());
    });

    it("pre-ticks a trusted file's commands", () => {
        render(() => (
            <LayoutPreviewModal preview={{ ...PREVIEW, trusted: true }} onOpen={() => Promise.resolve()} close={() => {}} />
        ));
        expect((screen.getByTestId("layout-preview-run-commands") as HTMLInputElement).checked).toBe(true);
        expect(screen.getByTestId("layout-preview-commands").textContent).not.toContain("wasn't saved");
    });

    it("opens in a new window by default, or adds to this window when chosen", async () => {
        const onOpen = vi.fn(() => Promise.resolve());
        render(() => <LayoutPreviewModal preview={PREVIEW} onOpen={onOpen} close={() => {}} />);
        expect((screen.getByTestId("layout-preview-new-window") as HTMLInputElement).checked).toBe(true);
        expect(screen.getByText("Opens 1 tab in a new window.")).toBeTruthy();

        fireEvent.click(screen.getByTestId("layout-preview-this-window"));
        expect(screen.getByText("Adds 1 tab to this window. Nothing that's open is replaced.")).toBeTruthy();
        fireEvent.click(screen.getByText("Open"));
        await vi.waitFor(() => expect(onOpen).toHaveBeenCalledWith({ runCommands: false, newWindow: false }));
    });
});
