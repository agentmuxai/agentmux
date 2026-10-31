// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Two Editor panes that end up as tabs of one pane become one (editor-doc-tabs.ts `mergeFrom`,
 * doc-pane-merge.ts): the arriving Editor's document tabs join the destination's with their state,
 * and a document open in both is resolved without losing unsaved text.
 * docs/specs/SPEC_DOCUMENT_PANES_MERGE_DOCUMENT_TABS_2026_10_30.md §3.
 */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";

const h = vi.hoisted(() => ({
    files: new Map<string, string>(),
    /** Reads still running: a test ends only once they have, so none lands on a pane already disposed. */
    inflight: new Set<Promise<unknown>>(),
}));

vi.mock("@/app/store/global", () => ({
    pushNotification: () => {},
    setActiveTab: () => {},
    workspace: () => null,
    useBlockAtom: (_blockId: string, _key: string, make: () => unknown) => createRoot(() => make()),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        FileProbeCommand: vi.fn(async (_c: unknown, req: { path: string }) => {
            const content = h.files.get(req.path);
            if (content === undefined) throw new Error(`no such file: ${req.path}`);
            return { size: content.length, mtime: 0, kind: "text", mime: "text/plain", text: { encoding: "utf-8", bom: "none" }, read_only: false };
        }),
        ReadEditorFileCommand: vi.fn((_c: unknown, req: { path: string }) => {
            const read = (async () => {
                const content = h.files.get(req.path);
                if (content === undefined) throw new Error(`no such file: ${req.path}`);
                return { content, read_only: false, encoding: "UTF-8", bom: "none", line_ending: "lf" };
            })();
            h.inflight.add(read);
            void read.catch(() => {}).finally(() => h.inflight.delete(read));
            return read;
        }),
        CreateScratchFileCommand: vi.fn(async () => ({ file_path: "c:/scratch/s1.md", scratch_id: "s1", display_name: "Untitled-1" })),
        WatchEditorFileCommand: vi.fn(async () => {}),
        UnwatchEditorFileCommand: vi.fn(async () => {}),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/services", () => ({ WorkspaceService: {} }));
vi.mock("@/app/tab/tab-presets", () => ({ createBlockOnModel: () => {}, waitForLayoutModel: async () => null }));
vi.mock("@/app/store/mps", () => {
    const mock = { muxEventSubscribe: () => () => {} };
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        (mock.muxEventSubscribe as (sub: unknown) => () => void)({ eventType, scope, handler: (e: any) => { if (e?.data != null) handler(e.data, e); } } as any);
    return { ...mock, onMuxEvent };
});
vi.mock("@/app/drag/file-drop", () => ({ registerFileDropTarget: () => () => {} }));
vi.mock("@/app/drag/file-drop-actions", () => ({ notifyDrop: { cantOpen: () => {}, cantMove: () => {} } }));
vi.mock("@/app/util/reveal-block", () => ({ showBlockWithoutFocus: async () => {} }));

import { canMergeDocPanes, mergeArrivingDocumentPanes } from "@/app/doc-tabs/doc-pane-merge";
import { docTabHost, registerDocTabHost } from "@/app/doc-tabs/doc-tab-hosts";
import { snapshot } from "@/app/store/editor-pane-state-store";
import { editorDocTabHost, trackSave } from "./editor-doc-tabs";
import { EditorViewModel } from "./editor-model";

let blocks = 0;
const cleanups: (() => void)[] = [];
function mount(meta: Record<string, unknown> = {}) {
    const [m, setM] = createSignal<Record<string, unknown>>(meta);
    const ctx: PaneTabHostContext = {
        blockId: `mg${++blocks}`,
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
const pathsOf = (m: EditorViewModel) => tabsOf(m).map((t) => t.filePath);

async function openLoaded(model: EditorViewModel, path: string) {
    await model.openFile(path);
    await until(() => model.contentAtom() === h.files.get(path));
}

const mergeInto = async (into: EditorViewModel, from: EditorViewModel) => docTabHost(into.blockId)!.mergeFrom!(docTabHost(from.blockId)!);

beforeEach(() => {
    h.files = new Map([
        ["c:/repo/a.ts", "const a = 1;\n"],
        ["c:/repo/b.ts", "const b = 2;\n"],
        ["c:/repo/c.ts", "const c = 3;\n"],
        ["c:/scratch/s1.md", ""],
    ]);
});
afterEach(async () => {
    for (let quiet = 0; quiet < 3; ) {
        if (h.inflight.size > 0) {
            quiet = 0;
            await Promise.allSettled([...h.inflight]);
        } else quiet++;
        await new Promise((r) => setTimeout(r, 5));
    }
    for (const c of cleanups.splice(0)) c();
});

describe("merging an Editor pane into another", () => {
    it("takes its document tabs after the destination's, with unsaved text, and keeps the destination's active", async () => {
        const dest = mount();
        const from = mount();
        await openLoaded(dest, "c:/repo/a.ts");
        const front = dest.activeIdAtom();
        await openLoaded(from, "c:/repo/b.ts");
        from.onContentChange("const b = 99; // edited\n");
        const edited = from.activeIdAtom()!;
        await openLoaded(from, "c:/repo/c.ts");

        expect(await mergeInto(dest, from)).toEqual({ merged: true });

        expect(pathsOf(dest)).toEqual(["c:/repo/a.ts", "c:/repo/b.ts", "c:/repo/c.ts"]);
        expect(tabsOf(from)).toEqual([]);
        expect(dest.activeIdAtom()).toBe(front);
        expect(tabsOf(dest).find((t) => t.id === edited)!.dirty).toBe(true);
        expect(dest._contentByTab.get(edited)).toBe("const b = 99; // edited\n");
    });

    it("the same file open and clean in both: one tab, the destination's", async () => {
        const dest = mount();
        const from = mount();
        await openLoaded(dest, "c:/repo/a.ts");
        const mine = dest.activeIdAtom();
        await openLoaded(from, "c:/repo/a.ts");

        expect(await mergeInto(dest, from)).toEqual({ merged: true });

        expect(tabsOf(dest).map((t) => t.id)).toEqual([mine]);
        expect(tabsOf(from)).toEqual([]);
    });

    it("an unsaved copy replaces the clean one, in its place", async () => {
        const dest = mount();
        const from = mount();
        await openLoaded(dest, "c:/repo/a.ts");
        await openLoaded(dest, "c:/repo/b.ts");
        await openLoaded(from, "c:/repo/a.ts");
        from.onContentChange("const a = 7; // unsaved\n");
        const unsaved = from.activeIdAtom()!;

        expect(await mergeInto(dest, from)).toEqual({ merged: true });

        expect(pathsOf(dest)).toEqual(["c:/repo/a.ts", "c:/repo/b.ts"]);
        expect(tabsOf(dest)[0].id).toBe(unsaved);
        expect(dest._contentByTab.get(unsaved)).toBe("const a = 7; // unsaved\n");
    });

    it("two different unsaved copies: the arriving text becomes an untitled tab; nothing is lost", async () => {
        const dest = mount();
        const from = mount();
        await openLoaded(dest, "c:/repo/a.ts");
        dest.onContentChange("mine\n");
        const mine = dest.activeIdAtom()!;
        await openLoaded(from, "c:/repo/a.ts");
        from.onContentChange("theirs\n");

        expect(await mergeInto(dest, from)).toEqual({ merged: true });
        expect(tabsOf(dest)).toHaveLength(2);

        expect(dest._contentByTab.get(mine)).toBe("mine\n");
        expect(dest.activeIdAtom()).toBe(mine); // the person stays on their own document
        const copy = tabsOf(dest).find((t) => t.id !== mine)!;
        expect(copy.isScratch).toBe(true);
        expect(dest._contentByTab.get(copy.id)).toBe("theirs\n");
        expect(tabsOf(from)).toEqual([]);
    });

    it("different unsaved copies whose second one can't be kept stop the merge, and nothing moves", async () => {
        const dest = mount();
        const from = mount();
        await openLoaded(dest, "c:/repo/a.ts");
        dest.onContentChange("mine\n");
        await openLoaded(from, "c:/repo/a.ts");
        from.onContentChange("binary\u0000data\n");
        await openLoaded(from, "c:/repo/b.ts");

        const result = await mergeInto(dest, from);

        expect(result.merged).toBe(false);
        expect(tabsOf(dest)).toHaveLength(1);
        expect(tabsOf(from)).toHaveLength(2);
        expect(from._contentByTab.size).toBeGreaterThan(0);
    });

    it("two identical unsaved copies are one tab", async () => {
        const dest = mount();
        const from = mount();
        await openLoaded(dest, "c:/repo/a.ts");
        dest.onContentChange("same\n");
        await openLoaded(from, "c:/repo/a.ts");
        from.onContentChange("same\n");

        expect(await mergeInto(dest, from)).toEqual({ merged: true });

        expect(tabsOf(dest)).toHaveLength(1);
        expect(tabsOf(from)).toEqual([]);
    });

    it("a tab being saved in the arriving Editor stops the merge, and nothing moves", async () => {
        const dest = mount();
        const from = mount();
        await openLoaded(from, "c:/repo/b.ts");
        let finish!: () => void;
        const saved = trackSave(from, from.activeIdAtom(), new Promise<void>((r) => (finish = r)));

        expect(await mergeInto(dest, from)).toEqual({ merged: false, reason: "It is still being saved. Try again in a moment." });
        expect(tabsOf(from)).toHaveLength(1);
        expect(tabsOf(dest)).toEqual([]);
        finish();
        await saved;
    });

    it("an Editor on a host is not merged with one on this computer", () => {
        const local = mount();
        const remote = mount({ connection: "user@box" });
        expect(canMergeDocPanes(docTabHost(local.blockId), docTabHost(remote.blockId))).toBe(false);
        expect(canMergeDocPanes(docTabHost(local.blockId), docTabHost(mount().blockId))).toBe(true);
    });

    it("merges into the target, answers the emptied pane, and leaves a pane of another computer", async () => {
        const dest = mount();
        const from = mount();
        const remote = mount({ connection: "user@box" });
        await openLoaded(from, "c:/repo/b.ts");

        const done = await mergeArrivingDocumentPanes(dest.blockId, [dest.blockId, from.blockId, remote.blockId], [from.blockId, remote.blockId]);

        expect(done).toEqual([from.blockId]);
        expect(pathsOf(dest)).toEqual(["c:/repo/b.ts"]);
    });

    it("two Editors arriving together, with none in the stack, merge into the first", async () => {
        const first = mount();
        const second = mount();
        await openLoaded(first, "c:/repo/a.ts");
        await openLoaded(second, "c:/repo/b.ts");

        const done = await mergeArrivingDocumentPanes(first.blockId, [first.blockId, second.blockId], [first.blockId, second.blockId]);

        expect(done).toEqual([second.blockId]);
        expect(pathsOf(first)).toEqual(["c:/repo/a.ts", "c:/repo/b.ts"]);
    });
});
