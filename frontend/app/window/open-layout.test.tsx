// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// ☰ → Layouts → "Open layout…" (SPEC_LAYOUT_FILES_2026_09_25.md §3.5).

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LayoutPreviewResult } from "@/types/rpc/LayoutPreviewResult";

const h = vi.hoisted(() => ({
    showOpenLayoutDialog: vi.fn<() => Promise<string | null>>(),
    PreviewLayoutCommand: vi.fn(),
    OpenLayoutCommand: vi.fn(),
    pushNotification: vi.fn(),
    openModal: vi.fn(),
}));

vi.mock("@/store/global", () => ({
    getApi: () => ({ showOpenLayoutDialog: h.showOpenLayoutDialog }),
    pushNotification: h.pushNotification,
}));
vi.mock("@/app/store/modalmodel", () => ({ openModal: h.openModal }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { PreviewLayoutCommand: h.PreviewLayoutCommand, OpenLayoutCommand: h.OpenLayoutCommand },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: { tag: "tab-client" } }));
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

        h.OpenLayoutCommand.mockResolvedValue({ tab_ids: ["t1"], notes: [] });
        await props.onOpen(true);
        expect(h.OpenLayoutCommand).toHaveBeenCalledWith(
            { tag: "tab-client" },
            { path: "/l/work.agentmux-layout.json", window_id: "win-7", run_commands: true },
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
    it("says what didn't come across as a warning", async () => {
        h.OpenLayoutCommand.mockResolvedValue({ tab_ids: ["t1", "t2"], notes: ["An agent didn't start"] });
        await applyLayout("/l/x.agentmux-layout.json", false);
        const notif = h.pushNotification.mock.calls[0][0];
        expect(notif.type).toBe("warning");
        expect(notif.title).toContain("2 tabs added");
        expect(notif.message).toContain("An agent didn't start");
    });

    it("reports a failed open", async () => {
        h.OpenLayoutCommand.mockRejectedValue(new Error("no such window"));
        await applyLayout("/l/x.agentmux-layout.json", false);
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
        await vi.waitFor(() => expect(onOpen).toHaveBeenCalledWith(true));
        await vi.waitFor(() => expect(close).toHaveBeenCalled());
    });

    it("pre-ticks a trusted file's commands", () => {
        render(() => (
            <LayoutPreviewModal preview={{ ...PREVIEW, trusted: true }} onOpen={() => Promise.resolve()} close={() => {}} />
        ));
        expect((screen.getByTestId("layout-preview-run-commands") as HTMLInputElement).checked).toBe(true);
        expect(screen.getByTestId("layout-preview-commands").textContent).not.toContain("wasn't saved");
    });
});
