// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Moving an Editor tab to another Editor pane (editor-doc-tabs.ts): its text,
 * unsaved edits included, its encoding, its file watch and its undo history
 * go with it; a tab never moves between this computer and a host, and a tab
 * with unsaved edits never gives way to the same file open in the target.
 * docs/reports/REPORT_DOC_TAB_DRAG_AND_DROP_2026_10_09.md §3.4.
 */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";

const h = vi.hoisted(() => ({
    files: new Map<string, string>(),
    reads: [] as string[],
    watched: [] as string[],
    unwatched: [] as string[],
}));

vi.mock("@/app/store/global", () => ({
    pushNotification: () => {},
    setActiveTab: () => {},
    workspace: () => null,
    useBlockAtom: (_blockId: string, _key: string, make: () => unknown) => createRoot(() => make()),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ReadEditorFileCommand: vi.fn(async (_c: unknown, req: { path: string }) => {
            h.reads.push(req.path);
            const content = h.files.get(req.path);
            if (content === undefined) throw new Error(`no such file: ${req.path}`);
            return { content, read_only: false, encoding: "UTF-16LE", bom: "utf16le", line_ending: "crlf" };
        }),
        WatchEditorFileCommand: vi.fn(async (_c: unknown, req: { path: string }) => void h.watched.push(req.path)),
        UnwatchEditorFileCommand: vi.fn(async (_c: unknown, req: { path: string }) => void h.unwatched.push(req.path)),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/services", () => ({ WorkspaceService: {} }));
vi.mock("@/app/tab/tab-presets", () => ({ createBlockOnModel: () => {}, waitForLayoutModel: async () => null }));
vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: () => () => {} }));
vi.mock("@/app/drag/file-drop", () => ({ registerFileDropTarget: () => () => {} }));
vi.mock("@/app/drag/file-drop-actions", () => ({ notifyDrop: { cantOpen: () => {}, cantMove: () => {} } }));
vi.mock("@/app/util/reveal-block", () => ({ showBlockWithoutFocus: async () => {} }));

import { moveDocTab, registerDocTabHost } from "@/app/doc-tabs/doc-tab-hosts";
import { dispatch, snapshot } from "@/app/store/editor-pane-state-store";
import { editorDocTabHost, provideEditorStates, takeMovedEditorState } from "./editor-doc-tabs";
import { EditorViewModel } from "./editor-model";

let blocks = 0;
const cleanups: (() => void)[] = [];
function mount(meta: Record<string, unknown> = {}) {
    const [m, setM] = createSignal<Record<string, unknown>>(meta);
    const ctx: PaneTabHostContext = {
        blockId: `mv${++blocks}`,
        meta: () => m() as MetaType,
        setMeta: async (patch) => void setM({ ...m(), ...patch }),
        isFocused: () => true,
        visibility: () => "active",
    };
    const model = new EditorViewModel(ctx);
    const off = registerDocTabHost(ctx.blockId, editorDocTabHost(model));
    cleanups.push(() => {
        off();
        model.dispose();
    });
    return model;
}

async function until(check: () => boolean, ms = 1000): Promise<void> {
    const end = Date.now() + ms;
    while (!check()) {
        if (Date.now() > end) throw new Error("timed out");
        await new Promise((r) => setTimeout(r, 10));
    }
}
const tabsOf = (m: EditorViewModel) => snapshot(m.blockId)?.tabs ?? [];

async function openLoaded(model: EditorViewModel, path: string) {
    await model.openFile(path);
    await until(() => model.contentAtom() === h.files.get(path));
}

beforeEach(() => {
    h.files = new Map([
        ["c:/repo/a.ts", "const a = 1;\n"],
        ["c:/repo/b.ts", "const b = 2;\n"],
    ]);
    h.reads = [];
    h.watched = [];
    h.unwatched = [];
});
afterEach(() => {
    for (const c of cleanups.splice(0)) c();
});

describe("moving an Editor tab to another Editor", () => {
    it("takes its unsaved text, its unsaved mark, its encoding and its file watch", async () => {
        const a = mount();
        const b = mount();
        await openLoaded(a, "c:/repo/a.ts");
        a.onContentChange("const a = 42; // edited\n");
        const id = a.activeIdAtom()!;

        expect(moveDocTab(a.blockId, id, b.blockId)).toEqual({ moved: true });

        expect(tabsOf(a)).toEqual([]);
        expect(a._contentByTab.has(id)).toBe(false);
        expect(h.unwatched).toContain("c:/repo/a.ts");
        const moved = tabsOf(b).find((t) => t.id === id)!;
        expect(moved.dirty).toBe(true);
        expect(b.activeIdAtom()).toBe(id);
        expect(b.contentAtom()).toBe("const a = 42; // edited\n");
        expect(b._encodingByTab.get(id)).toMatchObject({ encoding: "UTF-16LE", lineEnding: "crlf" });
        expect(h.watched.filter((p) => p === "c:/repo/a.ts")).toHaveLength(2); // once in each pane
        expect(h.reads.filter((p) => p === "c:/repo/a.ts")).toHaveLength(1); // not read again
        // Moved, not closed: the source can't reopen it.
        expect(snapshot(a.blockId)?.recentlyClosed).toEqual([]);
    });

    it("a tab not read yet moves without its text, and the target reads it", async () => {
        const a = mount({ doctabs: { v: 1, active: 0, tabs: [{ key: "c:/repo/b.ts", title: "b.ts", state: { path: "c:/repo/b.ts", language: "typescript" } }, { key: "c:/repo/a.ts", title: "a.ts", state: { path: "c:/repo/a.ts", language: "typescript" } }] } });
        const b = mount();
        await until(() => a.contentAtom() === h.files.get("c:/repo/b.ts"));
        const unread = tabsOf(a).find((t) => t.filePath === "c:/repo/a.ts")!;
        expect(unread.contentLoaded).toBe(false);
        moveDocTab(a.blockId, unread.id, b.blockId);
        await until(() => b.contentAtom() === h.files.get("c:/repo/a.ts"));
    });

    it("hands the target view its undo history once", async () => {
        const a = mount();
        const b = mount();
        await openLoaded(a, "c:/repo/a.ts");
        const id = a.activeIdAtom()!;
        const json = { doc: "const a = 1;\n", selection: { ranges: [{ anchor: 3, head: 3 }], main: 0 }, history: { done: [], undone: [] } };
        provideEditorStates(a, (tabId) => (tabId === id ? ({ toJSON: () => json } as never) : undefined));
        moveDocTab(a.blockId, id, b.blockId);
        expect(takeMovedEditorState(b, id)).toBe(json);
        expect(takeMovedEditorState(b, id)).toBeUndefined();
    });

    it("the same file open in the target: a clean tab gives way to it, a dirty one is refused", async () => {
        const a = mount();
        const b = mount();
        await openLoaded(a, "c:/repo/a.ts");
        await openLoaded(b, "c:/repo/a.ts");
        await openLoaded(b, "c:/repo/b.ts");
        const bCopy = tabsOf(b).find((t) => t.filePath === "c:/repo/a.ts")!.id;

        const id = a.activeIdAtom()!;
        a.onContentChange("unsaved\n");
        const dirty = moveDocTab(a.blockId, id, b.blockId);
        expect(dirty.moved).toBe(false);
        expect(dirty).toMatchObject({ reason: expect.stringContaining("already open there") });
        expect(tabsOf(a)).toHaveLength(1);
        // The model's text map, not contentAtom: typing doesn't re-fire that
        // (CodeMirror holds the live text).
        expect(a._contentByTab.get(id)).toBe("unsaved\n");

        // Saved, so clean again: it gives way to the copy already there.
        dispatch(a.blockId, { type: "ClearDirty", tabId: id });
        expect(moveDocTab(a.blockId, id, b.blockId)).toEqual({ moved: true });
        expect(tabsOf(a)).toEqual([]);
        expect(tabsOf(b)).toHaveLength(2);
        expect(b.activeIdAtom()).toBe(bCopy);
    });

    it("never between this computer and a host", async () => {
        const a = mount();
        const remote = mount({ connection: "user@box" });
        await openLoaded(a, "c:/repo/a.ts");
        const result = moveDocTab(a.blockId, a.activeIdAtom()!, remote.blockId);
        expect(result).toEqual({ moved: false, reason: "Its file is on another computer." });
        expect(tabsOf(a)).toHaveLength(1);
        expect(tabsOf(remote)).toEqual([]);
    });
});
