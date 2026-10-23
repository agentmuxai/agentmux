// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The dialogs of ☰ → Workspaces (SPEC_WORKSPACES_2026_10_23.md §3.2).

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { UserWorkspaceInfo, WorkspaceImportPreview } from "@/app/store/services";
import {
    DeleteWorkspaceConfirm,
    deleteWorkspaceText,
    ImportPreviewModal,
    WorkspaceNameModal,
    WorkspacePickerModal,
} from "./workspace-modals";

afterEach(() => cleanup());

const ws = (id: string, name: string, current = false, windows = 2, closed = 1): UserWorkspaceInfo => ({
    id,
    name,
    current,
    opened_at: 1,
    windows,
    closed,
});

describe("the name prompt", () => {
    it("starts with the suggested name and submits the trimmed one on Enter", async () => {
        const onSubmit = vi.fn();
        const close = vi.fn();
        render(() => <WorkspaceNameModal title="New workspace" initial="Workspace 2" confirmLabel="Create" onSubmit={onSubmit} close={close} />);
        const input = (await screen.findByTestId("workspace-name-input")) as HTMLInputElement;
        expect(input.value).toBe("Workspace 2");
        fireEvent.input(input, { target: { value: "  Ops  " } });
        fireEvent.keyDown(input, { key: "Enter" });
        expect(close).toHaveBeenCalled();
        expect(onSubmit).toHaveBeenCalledWith("Ops");
    });

    it("won't take a blank name", async () => {
        const onSubmit = vi.fn();
        render(() => <WorkspaceNameModal title="Rename workspace" initial="" confirmLabel="Rename" onSubmit={onSubmit} close={() => {}} />);
        const button = await screen.findByRole("button", { name: "Rename" });
        expect((button as HTMLButtonElement).disabled).toBe(true);
        fireEvent.keyDown(await screen.findByTestId("workspace-name-input"), { key: "Enter" });
        expect(onSubmit).not.toHaveBeenCalled();
    });
});

describe("the workspace list", () => {
    it("shows each workspace's windows, and the open one can't be chosen", async () => {
        const onPick = vi.fn();
        const list = [ws("a", "Default Workspace", true), ws("b", "Reviews", false, 1, 0)];
        render(() => <WorkspacePickerModal title="Open workspace" workspaces={list} onPick={onPick} close={() => {}} />);
        const items = (await screen.findAllByTestId("workspace-picker-item")) as HTMLButtonElement[];
        expect(items.map((b) => b.textContent)).toEqual(["Default Workspaceopen", "Reviews1 window"]);
        expect(items[0].disabled).toBe(true);
        fireEvent.click(items[1]);
        expect(onPick).toHaveBeenCalledWith(list[1]);
    });
});

describe("the delete confirmation", () => {
    it("says what goes and what stays", () => {
        expect(deleteWorkspaceText(ws("b", "Reviews"))).toEqual({
            title: "Delete “Reviews”?",
            description: "Its 3 windows and their tabs and panes are deleted. Its agents stay in My Agents, with their conversations.",
        });
    });

    it("deletes only when confirmed", async () => {
        const onConfirm = vi.fn(async () => {});
        render(() => <DeleteWorkspaceConfirm workspace={ws("b", "Reviews")} onConfirm={onConfirm} close={() => {}} />);
        fireEvent.click(await screen.findByRole("button", { name: "Delete workspace" }));
        await vi.waitFor(() => expect(onConfirm).toHaveBeenCalledTimes(1));
    });
});

describe("the import preview", () => {
    const preview: WorkspaceImportPreview = {
        name: "Ops",
        trusted: false,
        windows: 2,
        closed: 1,
        tabs: [{ name: "Work", panes: ["Terminal (command not run: `echo hi`)"] }],
        commands: ["echo hi"],
        notes: ["There's no agent “Ghost” here — its pane opens the agent picker."],
    };

    it("says what opens and that the open workspace closes first", async () => {
        render(() => <ImportPreviewModal preview={preview} current="Default Workspace" onImport={() => {}} close={() => {}} />);
        expect(await screen.findByText("Import “Ops”")).toBeTruthy();
        expect(screen.getByText(/Opens 2 windows \(1 closed\) and 1 tab as a new workspace\. Default Workspace closes first/)).toBeTruthy();
        expect(screen.getByTestId("import-preview-notes").textContent).toContain("Ghost");
    });

    it("runs a file's commands only when the box is ticked, unticked for a file from elsewhere", async () => {
        const onImport = vi.fn();
        render(() => <ImportPreviewModal preview={preview} current="W" onImport={onImport} close={() => {}} />);
        const box = (await screen.findByTestId("import-preview-run-commands")) as HTMLInputElement;
        expect(box.checked).toBe(false);
        fireEvent.click(screen.getByRole("button", { name: "Import" }));
        expect(onImport).toHaveBeenCalledWith(false);
    });

    it("ticks the box for a file this AgentMux saved", async () => {
        const onImport = vi.fn();
        render(() => <ImportPreviewModal preview={{ ...preview, trusted: true }} current="W" onImport={onImport} close={() => {}} />);
        expect(((await screen.findByTestId("import-preview-run-commands")) as HTMLInputElement).checked).toBe(true);
        fireEvent.click(screen.getByRole("button", { name: "Import" }));
        expect(onImport).toHaveBeenCalledWith(true);
    });
});
