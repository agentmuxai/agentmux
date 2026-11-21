// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A link to a local path opens a folder in Files and anything else in the Editor. srv reports a
// listing that failed in the result's `error`; it does not throw, so a file used to count as a folder
// and opened in Files as "This is a file, not a folder."

import { beforeEach, describe, expect, it, vi } from "vitest";

const fsList = vi.fn();
const agentFolder = vi.fn();
const pushNotification = vi.fn();
const paneOpen = vi.fn();
const openInEditorPaneOnScreen = vi.fn((..._args: unknown[]) => Promise.resolve(false));

vi.mock("@/app/store/global", () => ({
    atoms: { activeTabId: () => "tab-1" },
    pushNotification: (...a: unknown[]) => pushNotification(...a),
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        FsListCommand: (...a: unknown[]) => fsList(...a),
        FsAgentFolderCommand: (...a: unknown[]) => agentFolder(...a),
        PaneOpenCommand: (...a: unknown[]) => paneOpen(...a),
    },
}));
vi.mock("@/app/util/editor-open", () => ({
    openInEditorPaneOnScreen: (...a: unknown[]) => openInEditorPaneOnScreen(...a),
}));
vi.mock("@/app/util/open-kanban-task", () => ({ openKanbanTask: vi.fn() }));

import { openPathLink } from "./link-destinations";

const opened = () => (paneOpen.mock.calls[0]?.[1] as { view: string; file: string } | undefined)?.view;

beforeEach(() => {
    fsList.mockReset();
    agentFolder.mockReset();
    agentFolder.mockResolvedValue({ agent_folder: false });
    pushNotification.mockClear();
    paneOpen.mockReset();
    paneOpen.mockResolvedValue(undefined);
    openInEditorPaneOnScreen.mockReset();
    openInEditorPaneOnScreen.mockResolvedValue(false);
});

describe("openPathLink", () => {
    it("opens a folder in Files", async () => {
        fsList.mockResolvedValue({ path: "C:/work", entries: [], cursor: null });
        await openPathLink("C:/work");
        expect(opened()).toBe("files");
    });

    it("opens a file in the Editor when its listing comes back with an error (srv does not throw)", async () => {
        fsList.mockResolvedValue({
            path: "C:/work/notes.md",
            entries: [],
            error: { kind: "not_a_directory", message: "This isn't a folder." },
        });
        await openPathLink("C:/work/notes.md");
        expect(opened()).toBe("editor");
    });

    it("opens a path that is not there in the Editor, which says so", async () => {
        fsList.mockResolvedValue({ path: "C:/work/gone.md", entries: [], error: { kind: "not_found", message: "" } });
        await openPathLink("C:/work/gone.md");
        expect(opened()).toBe("editor");
    });

    it("opens a folder AgentMux may not read in Files, which says why", async () => {
        for (const kind of ["permission_denied", "os_blocked"]) {
            paneOpen.mockClear();
            fsList.mockResolvedValue({ path: "/locked", entries: [], error: { kind, message: "no" } });
            await openPathLink("/locked");
            expect(opened()).toBe("files");
        }
    });

    it("opens a path whose listing throws in the Editor", async () => {
        fsList.mockRejectedValue(new Error("rpc down"));
        await openPathLink("C:/work/notes.md");
        expect(opened()).toBe("editor");
    });

    it("takes a trailing slash, of either kind, as a folder without listing it", async () => {
        await openPathLink("C:/work/");
        await openPathLink("C:\\work\\");
        expect(paneOpen.mock.calls.map((c) => (c[1] as { view: string }).view)).toEqual(["files", "files"]);
        expect(fsList).not.toHaveBeenCalled();
    });

    it("tries the Editor on screen first for a file, and a Ctrl+click skips it", async () => {
        fsList.mockResolvedValue({ path: "C:/work/notes.md", entries: [], error: { kind: "not_a_directory", message: "" } });
        openInEditorPaneOnScreen.mockResolvedValueOnce(true);
        await openPathLink("C:/work/notes.md");
        expect(paneOpen).not.toHaveBeenCalled();
        await openPathLink("C:/work/notes.md", { floating: true });
        expect(openInEditorPaneOnScreen).toHaveBeenCalledTimes(1);
        expect(opened()).toBe("editor");
    });

    it("says the open came from a link, so srv can show what a container agent's folder holds as a copy", async () => {
        fsList.mockResolvedValue({ path: "C:/work/notes.md", entries: [], error: { kind: "not_a_directory", message: "" } });
        await openPathLink("C:/work/notes.md");
        await openPathLink("C:/work/");
        expect(paneOpen.mock.calls.map((c) => (c[1] as { from_link?: boolean }).from_link)).toEqual([true, true]);
    });

    it("never gives a file in a container agent's folder to the Editor on screen", async () => {
        fsList.mockResolvedValue({ path: "C:/agents/aria/notes.md", entries: [], error: { kind: "not_a_directory", message: "" } });
        agentFolder.mockResolvedValue({ agent_folder: true });
        openInEditorPaneOnScreen.mockResolvedValue(true);
        await openPathLink("C:/agents/aria/notes.md");
        expect(openInEditorPaneOnScreen).not.toHaveBeenCalled();
        expect(paneOpen.mock.calls[0]?.[1]).toMatchObject({ view: "editor", file: "C:/agents/aria/notes.md", from_link: true });
    });

    it("tells the person why when a link into a container agent's folder is refused", async () => {
        fsList.mockResolvedValue({ path: "C:/agents/aria/out", entries: [], error: { kind: "not_a_directory", message: "" } });
        agentFolder.mockResolvedValue({ agent_folder: true });
        paneOpen.mockRejectedValue(new Error("This link leads out of the agent's folder."));
        await expect(openPathLink("C:/agents/aria/out")).rejects.toThrow("leads out");
        expect(pushNotification).toHaveBeenCalledTimes(1);
        expect((pushNotification.mock.calls[0]?.[0] as { message: string }).message).toContain("leads out");
    });

    it("has srv open a link of a container agent's message from the agent's folders, and reads nothing itself", async () => {
        await openPathLink("/workspace/notes.md", { owner: { blockId: "pane-1", confined: true } });
        await vi.waitFor(() => expect(paneOpen).toHaveBeenCalledTimes(1));
        expect(paneOpen.mock.calls[0]?.[1]).toEqual({ view: "editor", file: "/workspace/notes.md", message_of: "pane-1", tab_id: "tab-1" });
        expect(fsList).not.toHaveBeenCalled();
        expect(agentFolder).not.toHaveBeenCalled();
        expect(openInEditorPaneOnScreen).not.toHaveBeenCalled();
    });

    it("opens nothing for a link of a confined message with no pane to name, and says so", async () => {
        await openPathLink("/workspace/notes.md", { owner: { confined: true } });
        expect(paneOpen).not.toHaveBeenCalled();
        expect(fsList).not.toHaveBeenCalled();
        expect(openInEditorPaneOnScreen).not.toHaveBeenCalled();
        expect(pushNotification).toHaveBeenCalledTimes(1);
    });

    it("opens a link as before when whose folder it is in can't be asked", async () => {
        fsList.mockResolvedValue({ path: "C:/work/notes.md", entries: [], error: { kind: "not_a_directory", message: "" } });
        agentFolder.mockRejectedValue(new Error("rpc down"));
        openInEditorPaneOnScreen.mockResolvedValueOnce(true);
        await openPathLink("C:/work/notes.md");
        expect(paneOpen).not.toHaveBeenCalled();
    });
});
