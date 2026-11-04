// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor view as a whole, mounted over a real EditorViewModel: the empty state, the file tree's
 * right-click menu and the language server's status chip. The pieces it is built from have tests of their own.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";

const h = vi.hoisted(() => ({
    files: new Map<string, string>(),
    reads: [] as string[],
    watched: [] as string[],
    unwatched: [] as string[],
    /** Reads still running: a test ends only once they have, so none lands on a pane already disposed. */
    inflight: new Set<Promise<unknown>>(),
    listings: new Map<string, { name: string; is_dir: boolean; is_symlink: boolean }[]>(),
    clip: vi.fn(async (..._a: unknown[]) => {}),
}));

vi.mock("@/app/store/global", () => ({
    pushNotification: () => {},
    setActiveTab: () => {},
    workspace: () => null,
    useBlockAtom: (_blockId: string, _key: string, make: () => unknown) => createRoot(() => make()),
}));
vi.mock("@/app/store/rpc-api", () => {
    const specific: Record<string, unknown> = {
        GetEditorRootsCommand: async () => ({ home: "/home/u", drives: [] }),
        ListEditorDirCommand: async (_c: unknown, req: { path: string }) => ({ path: req.path, entries: h.listings.get(req.path) ?? [] }),
        FileProbeCommand: async () => ({ size: 0, mtime: 0, kind: "text", mime: "text/plain", text: { encoding: "utf-8", bom: "none" }, read_only: false }),
    };
    return { RpcApi: new Proxy(specific, { get: (t, k) => (k in t ? t[k as string] : async () => ({})) }) };
});
vi.mock("@/util/clipboard", () => ({ writeText: (...a: unknown[]) => h.clip(...a) }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/services", () => ({ WorkspaceService: {} }));
vi.mock("@/app/tab/tab-presets", () => ({ createBlockOnModel: () => {}, waitForLayoutModel: async () => null }));
vi.mock("@/app/store/mps", () => {
    const mock = { muxEventSubscribe: () => () => {} };
    // As the real onMuxEvent: a typed subscription is a muxEventSubscribe whose handler gets the data.
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        (mock.muxEventSubscribe as (sub: unknown) => () => void)({ eventType, scope, handler: (e: any) => { if (e?.data != null) handler(e.data, e); } } as any);
    return { ...mock, onMuxEvent };
});
vi.mock("./open-from-remote-modal", () => ({ OpenFromRemoteModal: () => null }));
vi.mock("@/app/drag/file-drop", () => ({ registerFileDropTarget: () => () => {} }));
vi.mock("@/app/drag/file-drop-actions", () => ({ notifyDrop: { cantOpen: () => {}, cantMove: () => {} } }));
vi.mock("@/app/util/reveal-block", () => ({ showBlockWithoutFocus: async () => {} }));


import { EditorViewComponent } from "./editor-view";
import { EditorViewModel } from "./editor-model";

let blocks = 0;
const models: EditorViewModel[] = [];
function modelOf(): EditorViewModel {
    const [m, setM] = createSignal<Record<string, unknown>>({});
    const ctx: PaneTabHostContext = {
        blockId: `view${++blocks}`,
        meta: () => m() as MetaType,
        setMeta: async (patch) => void setM({ ...m(), ...patch }),
        isFocused: () => true,
        visibility: () => "active",
    };
    const model = new EditorViewModel(ctx);
    models.push(model);
    return model;
}

afterEach(() => {
    cleanup();
    for (const m of models.splice(0)) m.dispose();
});

const entry = (name: string, isDir = false) => ({ name, is_dir: isDir, is_symlink: false });
const flush = () => new Promise((r) => setTimeout(r, 0));

async function mountView() {
    h.listings.clear();
    h.listings.set("/home/u", [entry("src", true), entry("a.ts")]);
    h.listings.set("/home/u/src", [entry("b.ts")]);
    const model = modelOf();
    const view = render(() => <EditorViewComponent model={model} />);
    await flush();
    await flush();
    return { model, ...view };
}
const item = (name: string) => screen.getByRole("treeitem", { name }).querySelector(".file-tree-row")!;
const menuLabels = () => [...document.querySelectorAll(".ctx-menu > *")].map((n) => (n.classList.contains("ctx-menu-sep") ? "-" : n.querySelector(".ctx-menu-label")!.textContent));
const pick = (label: string) => {
    const row = [...document.querySelectorAll(".ctx-menu-item")].find((n) => n.querySelector(".ctx-menu-label")!.textContent === label)!;
    fireEvent.pointerDown(row);
};

describe("the empty editor", () => {
    it("lists the shortcuts worth knowing and offers to open from a remote host", async () => {
        const { container, model } = await mountView();
        const rows = [...container.querySelectorAll(".editor-empty-shortcut")].map((r) => r.querySelector(".editor-empty-label")!.textContent);
        expect(rows).toEqual(["Find", "Toggle .md preview", "Zoom"]);
        expect(container.querySelectorAll(".editor-empty-shortcut")[2].querySelectorAll("kbd")).toHaveLength(2);
        expect(model.openFromRemoteAtom()).toBe(false);
        fireEvent.click(screen.getByRole("button", { name: /Open from remote/ }));
        expect(model.openFromRemoteAtom()).toBe(true);
    });

    it("points at the file tree only while the tree is hidden", async () => {
        const { container, model } = await mountView();
        expect(container.querySelector(".editor-empty-hint")).toBeNull();
        await model.toggleTreeExpanded();
        expect(container.querySelector(".editor-empty-hint")!.textContent).toContain("Show the file tree");
    });
});

describe("the file tree's right-click menu", () => {
    it("on a file: opens it three ways, copies its path, renames, deletes", async () => {
        await mountView();
        fireEvent.contextMenu(item("a.ts"));
        expect(menuLabels()).toEqual(["Open", "Open to the Side", "Open in New Tab", "-", "Copy Path", "Copy Relative Path", "-", "Reveal in Explorer", "-", "Rename…", "Delete"]);
        expect([...document.querySelectorAll(".ctx-menu-shortcut")].map((n) => n.textContent)).toEqual(["F2"]);
        expect(document.querySelector(".ctx-menu-item--danger .ctx-menu-label")!.textContent).toBe("Delete");
    });

    it("copies the path, and the path relative to its root", async () => {
        await mountView();
        fireEvent.contextMenu(item("a.ts"));
        pick("Copy Path");
        expect(h.clip).toHaveBeenLastCalledWith("/home/u/a.ts");
        expect(document.querySelector(".ctx-menu")).toBeNull();
        fireEvent.contextMenu(item("a.ts"));
        pick("Copy Relative Path");
        expect(h.clip).toHaveBeenLastCalledWith("a.ts");
    });

    it("opens the file where each entry says", async () => {
        const { model } = await mountView();
        const open = vi.spyOn(model, "openFile").mockResolvedValue(undefined as never);
        const side = vi.spyOn(model, "openToTheSide").mockResolvedValue(undefined as never);
        const tab = vi.spyOn(model, "openInNewTab").mockResolvedValue(undefined as never);
        for (const [label, spy] of [["Open", open], ["Open to the Side", side], ["Open in New Tab", tab]] as const) {
            fireEvent.contextMenu(item("a.ts"));
            pick(label);
            expect(spy).toHaveBeenCalledWith("/home/u/a.ts");
        }
    });

    it("asks before deleting a file, and a folder says it takes its contents", async () => {
        const { model } = await mountView();
        const del = vi.spyOn(model, "deleteFile").mockImplementation(async (_p, _d, ask) => void ask?.(() => {}));
        fireEvent.contextMenu(item("a.ts"));
        pick("Delete");
        expect(del).toHaveBeenLastCalledWith("/home/u/a.ts", false, expect.any(Function));
        expect(document.body.textContent).toContain('Delete "a.ts"?');
        expect(document.body.textContent).toContain('This will permanently delete "a.ts". This cannot be undone.');
        fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
        fireEvent.contextMenu(item("src"));
        pick("Delete");
        expect(del).toHaveBeenLastCalledWith("/home/u/src", true, expect.any(Function));
        expect(document.body.textContent).toContain('Delete folder "src"?');
        expect(document.body.textContent).toContain('This will permanently delete "src" and all its contents. This cannot be undone.');
    });

    it("on a folder: new entries, terminal, reveal, collapse, rename and delete; on the root no rename or delete", async () => {
        await mountView();
        fireEvent.contextMenu(item("src"));
        expect(menuLabels()).toEqual(["New File…", "New Folder…", "-", "Open in Terminal", "Reveal in Explorer", "Collapse Folder", "-", "Rename…", "Delete"]);
        fireEvent.keyDown(document, { key: "Escape" });
        fireEvent.contextMenu(item("u"));
        expect(menuLabels()).toEqual(["New File…", "New Folder…", "-", "Open in Terminal", "Reveal in Explorer", "Collapse Folder"]);
    });

    it("on the tree's background: new entries in the home folder and a refresh", async () => {
        const { model } = await mountView();
        const expand = vi.spyOn(model.treeModel, "expandFolder");
        fireEvent.contextMenu(screen.getByRole("tree"));
        expect(menuLabels()).toEqual(["New File…", "New Folder…", "-", "Refresh"]);
        pick("New File…");
        expect(expand).toHaveBeenCalledWith("/home/u");
        expect(document.querySelector(".file-tree-inline-input")).not.toBeNull();
    });

    it("renames in place from the menu", async () => {
        const { model } = await mountView();
        const rename = vi.spyOn(model, "renameFile").mockResolvedValue(undefined as never);
        fireEvent.contextMenu(item("a.ts"));
        pick("Rename…");
        const input = document.querySelector<HTMLInputElement>(".file-tree-inline-input")!;
        expect(input).not.toBeNull();
        fireEvent.input(input, { target: { value: "c.ts" } });
        fireEvent.keyDown(input, { key: "Enter" });
        await flush();
        expect(rename).toHaveBeenCalledWith("/home/u/a.ts", "c.ts");
    });

    it("on a tree with no home folder, offers only a refresh", async () => {
        const { model } = await mountView();
        model.treeModel.rootsAtom = () => [];
        fireEvent.contextMenu(screen.getByRole("tree"));
        expect(menuLabels()).toEqual(["Refresh"]);
    });
});
