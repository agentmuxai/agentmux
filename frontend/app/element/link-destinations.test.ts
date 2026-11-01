// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A link to a local path opens a folder in Files and anything else in the Editor. srv reports a
// listing that failed in the result's `error`; it does not throw, so a file used to count as a folder
// and opened in Files as "This is a file, not a folder."

import { beforeEach, describe, expect, it, vi } from "vitest";

const fsList = vi.fn();
const paneOpen = vi.fn((..._args: unknown[]) => Promise.resolve());
const openInEditorPaneOnScreen = vi.fn((..._args: unknown[]) => Promise.resolve(false));

vi.mock("@/app/store/global", () => ({ atoms: { activeTabId: () => "tab-1" } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        FsListCommand: (...a: unknown[]) => fsList(...a),
        PaneOpenCommand: (...a: unknown[]) => paneOpen(...a),
    },
}));
vi.mock("@/app/view/editor/editor-open", () => ({
    openInEditorPaneOnScreen: (...a: unknown[]) => openInEditorPaneOnScreen(...a),
}));
vi.mock("@/app/view/kanban/kanban-held", () => ({ openKanbanTask: vi.fn() }));

import { openPathLink } from "./link-destinations";

const opened = () => (paneOpen.mock.calls[0]?.[1] as { view: string; file: string } | undefined)?.view;

beforeEach(() => {
    fsList.mockReset();
    paneOpen.mockClear();
    openInEditorPaneOnScreen.mockClear();
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
});
