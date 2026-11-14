// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Renaming a file from its document tab (editor-tab-rename.ts): the rename on
 * disk and the tabs that follow it, in this pane and in others, and the name
 * box's state. A saved file is renamed in its folder; an Untitled document is
 * saved under the typed name. Nothing happens before a commit, and a refusal
 * keeps the box open with srv's sentence.
 * docs/specs/SPEC_EDITOR_TAB_RENAME_2026_11_14.md §4, §5, §11.
 */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";

const h = vi.hoisted(() => ({
    files: new Map<string, string>(),
    watched: [] as string[],
    unwatched: [] as string[],
    renames: [] as { path: string; new_name: string; connection?: string; block_id?: string }[],
    moves: [] as string[],
    /** What the next save of an Untitled document is refused with. */
    failMove: null as string | null,
    /** When set, a rename waits for it: a slow host. */
    gate: null as Promise<void> | null,
    subs: [] as { eventType: string; handler: (e: { data: unknown }) => void }[],
    scratch: 0,
    inflight: new Set<Promise<unknown>>(),
}));

const HOME = "c:/home";
const canon = (p: string) => p.replace(/\\/g, "/").replace(/^([A-Z]):/, (_, d: string) => d.toLowerCase() + ":");
const asWindows = (p: string) => p.replace(/\//g, "\\").replace(/^([a-z]):/, (_, d: string) => d.toUpperCase() + ":");
const emit = (eventType: string, data: unknown) => {
    for (const sub of [...h.subs]) if (sub.eventType === eventType) sub.handler({ data });
};

vi.mock("@/app/store/global", () => ({
    pushNotification: () => {},
    setActiveTab: () => {},
    workspace: () => null,
    useBlockAtom: (_blockId: string, _key: string, make: () => unknown) => createRoot(() => make()),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        FileProbeCommand: vi.fn(async (_c: unknown, req: { path: string }) => {
            const content = h.files.get(canon(req.path));
            if (content === undefined) throw new Error(`no such file: ${req.path}`);
            return { size: content.length, mtime: 0, kind: "text", mime: "text/plain", text: { encoding: "utf-8", bom: "none" }, read_only: false };
        }),
        ReadEditorFileCommand: vi.fn((_c: unknown, req: { path: string }) => {
            const read = (async () => {
                const content = h.files.get(canon(req.path));
                if (content === undefined) throw new Error(`no such file: ${req.path}`);
                return { content, read_only: false, encoding: "UTF-8", bom: "none", line_ending: "lf" };
            })();
            h.inflight.add(read);
            void read.catch(() => {}).finally(() => h.inflight.delete(read));
            return read;
        }),
        WriteEditorFileCommand: vi.fn(async (_c: unknown, req: { path: string; content: string }) => void h.files.set(canon(req.path), req.content)),
        WatchEditorFileCommand: vi.fn(async (_c: unknown, req: { path: string }) => void h.watched.push(req.path)),
        UnwatchEditorFileCommand: vi.fn(async (_c: unknown, req: { path: string }) => void h.unwatched.push(req.path)),
        ListEditorDirCommand: vi.fn(async (_c: unknown, req: { path: string }) => ({ path: req.path, entries: [] })),
        // As srv's `fs.rename`: never over an existing name, the new path in the system's own
        // spelling, and one `fs:renamed` event to every window.
        FsRenameCommand: vi.fn(async (_c: unknown, req: { path: string; new_name: string; connection?: string; block_id?: string }) => {
            h.renames.push(req);
            if (h.gate) await h.gate;
            const from = canon(req.path);
            const to = from.slice(0, from.lastIndexOf("/") + 1) + req.new_name;
            const moving = [...h.files.keys()].filter((p) => p === from || p.startsWith(from + "/"));
            if (moving.length === 0) throw new Error("It doesn't exist anymore.");
            if (h.files.has(to) && to.toLowerCase() !== from.toLowerCase()) throw new Error(`Something named “${req.new_name}” already exists here.`);
            for (const p of moving) {
                const content = h.files.get(p)!;
                h.files.delete(p);
                h.files.set(to + p.slice(from.length), content);
            }
            const new_path = req.connection ? to : asWindows(to);
            emit("fs:renamed", { old_path: req.connection ? from : asWindows(from), new_path, connection: req.connection ?? null });
            return { new_path };
        }),
        CreateScratchFileCommand: vi.fn(async () => {
            const scratch_id = `s${++h.scratch}`;
            const file_path = `${HOME}/.agentmux/cache/scratch/${scratch_id}.md`;
            h.files.set(file_path, "");
            return { file_path, scratch_id, display_name: `Untitled-${h.scratch}` };
        }),
        MoveScratchFileCommand: vi.fn(async (_c: unknown, req: { scratch_id: string; destination_path: string }) => {
            h.moves.push(req.destination_path);
            if (h.failMove) throw new Error(h.failMove);
            const from = `${HOME}/.agentmux/cache/scratch/${req.scratch_id}.md`;
            const to = canon(req.destination_path.replace(/^~/, HOME));
            if (h.files.has(to)) throw new Error(`Something named “${to.slice(to.lastIndexOf("/") + 1)}” already exists here.`);
            h.files.set(to, h.files.get(from) ?? "");
            h.files.delete(from);
            return { file_path: asWindows(to) };
        }),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/services", () => ({ WorkspaceService: {} }));
vi.mock("@/app/tab/tab-presets", () => ({ createBlockOnModel: () => {}, waitForLayoutModel: async () => null }));
vi.mock("@/app/store/mps", () => {
    const muxEventSubscribe = (sub: { eventType: string; handler: (e: { data: unknown }) => void }) => {
        h.subs.push(sub);
        return () => void h.subs.splice(h.subs.indexOf(sub), 1);
    };
    // As the real onMuxEvent: a typed subscription is a muxEventSubscribe whose handler gets the data.
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        muxEventSubscribe({ eventType, scope, handler: (e: any) => { if (e?.data != null) handler(e.data, e); } } as any);
    return { muxEventSubscribe, onMuxEvent };
});
vi.mock("@/app/drag/file-drop", () => ({ registerFileDropTarget: () => () => {} }));
vi.mock("@/app/drag/file-drop-actions", () => ({ notifyDrop: { cantOpen: () => {}, cantMove: () => {} } }));
vi.mock("@/app/util/reveal-block", () => ({ showBlockWithoutFocus: async () => {} }));

import { dispatch, snapshot } from "@/app/store/editor-pane-state-store";
import { trackSave } from "./editor-doc-tabs";
import { EditorViewModel } from "./editor-model";
import { createTabRename, forgetSaveFolder, renameOnDisk, untitledFolder } from "./editor-tab-rename";

let blocks = 0;
const cleanups: (() => void)[] = [];
function mount(meta: Record<string, unknown> = {}) {
    const [m, setM] = createSignal<Record<string, unknown>>(meta);
    const ctx: PaneTabHostContext = {
        blockId: `rn${++blocks}`,
        meta: () => m() as MetaType,
        setMeta: async (patch) => void setM({ ...m(), ...patch }),
        isFocused: () => true,
        visibility: () => "active",
    };
    const model = new EditorViewModel(ctx);
    cleanups.push(() => model.dispose());
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

/** A name box on `model`, how often it handed the keyboard back to the document, and its clock. */
function boxOn(model: EditorViewModel) {
    const focused = { times: 0 };
    const clock = { ms: 0 };
    return { box: createRoot(() => createTabRename(model, { focusDocument: () => void focused.times++, now: () => clock.ms })), focused, clock };
}

beforeEach(() => {
    h.files = new Map([
        ["c:/repo/a.ts", "const a = 1;\n"],
        ["c:/repo/b.ts", "const b = 2;\n"],
        ["c:/repo/src/x/deep.ts", "export {};\n"],
        ["/srv/app/main.py", "print(1)\n"],
    ]);
    h.watched = [];
    h.unwatched = [];
    h.renames = [];
    h.moves = [];
    h.failMove = null;
    h.gate = null;
    h.scratch = 0;
    forgetSaveFolder();
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

describe("a rename on disk", () => {
    it("moves the tab to the new path with its id, its text and its unsaved mark, and its file watch", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        const before = tabsOf(m)[0];
        m.onContentChange("const a = 42; // edited\n");

        expect(await renameOnDisk(m, "c:/repo/a.ts", "renamed.ts")).toBeNull();

        const after = tabsOf(m)[0];
        expect(after.id).toBe(before.id);
        expect(after.filePath).toBe("c:/repo/renamed.ts");
        expect(after.dirty).toBe(true);
        expect(m._contentByTab.get(after.id)).toBe("const a = 42; // edited\n");
        expect(h.files.get("c:/repo/renamed.ts")).toBe("const a = 1;\n");
        expect(h.unwatched).toContain("c:/repo/a.ts");
        expect(h.watched[h.watched.length - 1]).toBe("c:/repo/renamed.ts");
        // The pane's own block id goes with the request: a pane confined to an agent's workspace is refused by it.
        expect(h.renames[0]).toMatchObject({ path: "c:/repo/a.ts", new_name: "renamed.ts", block_id: m.blockId });
    });

    it("gives the tab the language of a changed extension, and leaves it alone otherwise", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        const language = tabsOf(m)[0].language;
        await renameOnDisk(m, "c:/repo/a.ts", "a2.ts");
        expect(tabsOf(m)[0].language).toBe(language);
        await renameOnDisk(m, "c:/repo/a2.ts", "notes.md");
        expect(tabsOf(m)[0].language).toBe("markdown");
    });

    it("a change of case only is a rename like any other", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        expect(await renameOnDisk(m, "c:/repo/a.ts", "A.ts")).toBeNull();
        expect(tabsOf(m)[0].filePath).toBe("c:/repo/A.ts");
        expect(snapshot(m.blockId)?.doc.tabs[0].title).toBe("A.ts");
    });

    it("refused: srv's sentence comes back and nothing changes", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        expect(await renameOnDisk(m, "c:/repo/a.ts", "b.ts")).toBe("Something named “b.ts” already exists here.");
        expect(tabsOf(m)[0].filePath).toBe("c:/repo/a.ts");
        expect(h.files.get("c:/repo/b.ts")).toBe("const b = 2;\n");
        h.files.delete("c:/repo/a.ts");
        expect(await renameOnDisk(m, "c:/repo/a.ts", "c.ts")).toBe("It doesn't exist anymore.");
    });

    it("is followed by every other pane on that file, and not by a pane on a host", async () => {
        const acting = mount();
        const other = mount();
        const remote = mount({ connection: "user@box" });
        await openLoaded(acting, "c:/repo/a.ts");
        await openLoaded(other, "c:/repo/a.ts");
        await openLoaded(other, "c:/repo/b.ts");
        h.files.set("c:/repo/a.ts", h.files.get("c:/repo/a.ts")!);
        await remote.openFile("c:/repo/a.ts");
        const otherTab = tabsOf(other).find((t) => t.filePath === "c:/repo/a.ts")!;

        await renameOnDisk(acting, "c:/repo/a.ts", "renamed.ts");

        expect(tabsOf(other).find((t) => t.id === otherTab.id)?.filePath).toBe("c:/repo/renamed.ts");
        expect(tabsOf(other).map((t) => t.filePath)).toEqual(["c:/repo/renamed.ts", "c:/repo/b.ts"]);
        expect(tabsOf(remote)[0].filePath).toBe("c:/repo/a.ts");
    });

    it("on a host: the request names the connection, and only panes on that host follow", async () => {
        const host = mount({ connection: "user@box" });
        const sameHost = mount({ connection: "user@box" });
        const otherHost = mount({ connection: "user@elsewhere" });
        const local = mount();
        for (const m of [host, sameHost, otherHost]) await m.openFile("/srv/app/main.py");
        await openLoaded(local, "/srv/app/main.py");

        expect(await renameOnDisk(host, "/srv/app/main.py", "app.py")).toBeNull();

        expect(h.renames[0]).toMatchObject({ connection: "user@box", block_id: host.blockId });
        expect(tabsOf(host)[0].filePath).toBe("/srv/app/app.py");
        expect(tabsOf(sameHost)[0].filePath).toBe("/srv/app/app.py");
        expect(tabsOf(otherHost)[0].filePath).toBe("/srv/app/main.py");
        expect(tabsOf(local)[0].filePath).toBe("/srv/app/main.py");
    });

    it("a folder renamed elsewhere (the Files pane) moves the tabs inside it", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/src/x/deep.ts");
        await openLoaded(m, "c:/repo/a.ts");
        emit("fs:renamed", { old_path: "C:\\repo\\src", new_path: "C:\\repo\\lib", connection: null });
        expect(tabsOf(m).map((t) => t.filePath)).toEqual(["c:/repo/lib/x/deep.ts", "c:/repo/a.ts"]);
    });

    it("a pane that already has a tab on the new path keeps its tab where it is", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        await openLoaded(m, "c:/repo/b.ts");
        // b.ts was deleted outside and its tab is still open, then a.ts takes its name.
        emit("fs:renamed", { old_path: "C:\\repo\\a.ts", new_path: "C:\\repo\\b.ts", connection: null });
        expect(tabsOf(m).map((t) => t.filePath)).toEqual(["c:/repo/a.ts", "c:/repo/b.ts"]);
    });

    it("a closed tab reopens on the new path", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        await openLoaded(m, "c:/repo/b.ts");
        m.closeTab(tabsOf(m).find((t) => t.filePath === "c:/repo/a.ts")!.id);
        await renameOnDisk(m, "c:/repo/a.ts", "renamed.ts");
        m.reopenLastClosed();
        expect(tabsOf(m).map((t) => t.filePath)).toContain("c:/repo/renamed.ts");
    });
});

describe("the name box of a saved file", () => {
    it("opens on the tab in front with its name, and a preview tab becomes a kept one", async () => {
        const m = mount();
        const { box } = boxOn(m);
        expect(box.start()).toBe(false);
        await m.openFilePreview("c:/repo/a.ts");
        await until(() => m.contentAtom() === h.files.get("c:/repo/a.ts"));
        expect(tabsOf(m)[0].isPreview).toBe(true);

        expect(box.start()).toBe(true);
        expect(box.tabId()).toBe(tabsOf(m)[0].id);
        expect(box.initialName()).toBe("a.ts");
        expect(box.line()).toBeNull();
        expect(tabsOf(m)[0].isPreview).toBe(false);
    });

    it("opens on a tab that isn't in front by bringing it forward", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        await openLoaded(m, "c:/repo/b.ts");
        const { box } = boxOn(m);
        const first = tabsOf(m)[0];
        expect(box.start(first.id)).toBe(true);
        expect(m.activeIdAtom()).toBe(first.id);
        expect(box.initialName()).toBe("a.ts");
    });

    it("renames on Enter, closes, and hands the keyboard back to the document", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        const { box, focused } = boxOn(m);
        box.start();
        await box.commit("  renamed.ts  ");
        expect(tabsOf(m)[0].filePath).toBe("c:/repo/renamed.ts");
        expect(h.renames[0].new_name).toBe("renamed.ts");
        expect(box.tabId()).toBeNull();
        expect(focused.times).toBe(1);
    });

    it("an empty or unchanged name closes the box and asks srv nothing", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        const { box, focused } = boxOn(m);
        for (const typed of ["", "   ", "a.ts", " a.ts "]) {
            box.start();
            await box.commit(typed);
            expect(box.tabId(), JSON.stringify(typed)).toBeNull();
        }
        expect(h.renames).toEqual([]);
        expect(focused.times).toBe(4);
    });

    it("a refused name keeps the box open with the reason, until the text changes", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        const { box, focused } = boxOn(m);
        box.start();
        await box.commit("b.ts");
        expect(box.tabId()).toBe(tabsOf(m)[0].id);
        expect(box.line()).toEqual({ kind: "error", text: "Something named “b.ts” already exists here." });
        expect(focused.times).toBe(0);
        box.setTyped("c.ts");
        expect(box.line()).toBeNull();
        await box.commit("c.ts");
        expect(box.tabId()).toBeNull();
        expect(tabsOf(m)[0].filePath).toBe("c:/repo/c.ts");
    });

    it("refuses by itself what no platform should take, and a name another tab here already has", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        await openLoaded(m, "c:/repo/b.ts");
        const { box } = boxOn(m);
        box.start(tabsOf(m)[0].id);
        await box.commit("a" + String.fromCharCode(0x202e) + "b.ts");
        expect(box.line()?.text).toBe("A name can't contain hidden direction characters.");
        // b.ts is gone from the disk but its tab is still open here.
        h.files.delete("c:/repo/b.ts");
        await box.commit("B.TS");
        expect(box.line()?.text).toBe("“B.TS” is already open in a tab here. Close that tab first.");
        expect(h.renames).toEqual([]);
    });

    it("waits out a save in flight", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        const { box } = boxOn(m);
        let done!: () => void;
        const save = trackSave(m, tabsOf(m)[0].id, new Promise<void>((r) => (done = r)));
        box.start();
        await box.commit("renamed.ts");
        expect(box.line()?.text).toBe("It is still being saved. Try again in a moment.");
        expect(h.renames).toEqual([]);
        done();
        await save;
        await box.commit("renamed.ts");
        expect(tabsOf(m)[0].filePath).toBe("c:/repo/renamed.ts");
    });

    it("while the answer is on its way: says so, takes no second Enter, and can't be cancelled", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        const { box } = boxOn(m);
        let open!: () => void;
        h.gate = new Promise<void>((r) => (open = r));
        box.start();
        const first = box.commit("renamed.ts");
        await until(() => h.renames.length === 1);
        expect(box.busy()).toBe(true);
        expect(box.line()).toEqual({ kind: "busy", text: "Renaming…" });
        await box.commit("other.ts");
        box.cancel(true);
        expect(box.tabId()).toBe(tabsOf(m)[0].id);
        expect(box.start()).toBe(false);
        open();
        await first;
        expect(h.renames).toHaveLength(1);
        expect(box.tabId()).toBeNull();
        expect(tabsOf(m)[0].filePath).toBe("c:/repo/renamed.ts");
    });

    it("cancel closes it and changes nothing; only Escape hands the keyboard back", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        const { box, focused } = boxOn(m);
        box.start();
        box.cancel();
        expect(box.tabId()).toBeNull();
        expect(focused.times).toBe(0);
        box.start();
        box.cancel(true);
        expect(focused.times).toBe(1);
        expect(h.renames).toEqual([]);
    });

    it("a double-click that only followed a tab closing renames nothing", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        await openLoaded(m, "c:/repo/b.ts");
        const { box, clock } = boxOn(m);
        const [a, b] = tabsOf(m);
        // A double-click on a.ts's close button: the first click closes it, the second lands on b.ts.
        clock.ms = 1000;
        m.closeTab(a.id);
        clock.ms = 1150;
        expect(box.startFromDoubleClick(b.id)).toBe(false);
        expect(box.tabId()).toBeNull();
        // A double-click of its own, a moment later, is one.
        clock.ms = 1600;
        expect(box.startFromDoubleClick(b.id)).toBe(true);
        expect(box.tabId()).toBe(b.id);
    });

    it("a dropped item with no file behind it has nothing to rename", async () => {
        const m = mount();
        const { box } = boxOn(m);
        expect(box.refusal({ filePath: "agentmux-dropped:/1/photo.png", isScratch: false })).toBe("This document has no file on disk.");
        expect(box.refusal({ filePath: "c:/repo/a.ts", isScratch: false })).toBeNull();
        expect(box.refusal({ filePath: `${HOME}/.agentmux/cache/scratch/s1.md`, isScratch: true })).toBeNull();
        dispatch(m.blockId, { type: "OpenFile", path: "agentmux-dropped:/1/photo.png", language: "text", mode: "pinned", source: "user" });
        expect(box.start()).toBe(false);
    });
});

describe("the name box of an Untitled document", () => {
    it("shows where the name will be saved: the folder of the file the pane showed last", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/src/x/deep.ts");
        await openLoaded(m, "c:/repo/a.ts");
        const id = await m.openScratch(false);
        const { box } = boxOn(m);
        expect(untitledFolder(m)).toBe("c:/repo");

        expect(box.start()).toBe(true);
        expect(box.tabId()).toBe(id);
        expect(box.initialName()).toBe("Untitled-1");
        box.setTyped("notes.md");
        expect(box.line()).toEqual({ kind: "hint", text: "Saves to c:/repo/notes.md" });
        box.setTyped("drafts/notes.md");
        expect(box.line()).toEqual({ kind: "hint", text: "Saves to c:/repo/drafts/notes.md" });
        box.setTyped("~/elsewhere/notes.md");
        expect(box.line()).toEqual({ kind: "hint", text: "Saves to ~/elsewhere/notes.md" });
        box.setTyped("../notes.md");
        expect(box.line()).toBeNull();
    });

    it("saves the buffer under the name, and the tab becomes that file", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        const id = (await m.openScratch(false))!;
        m.onContentChange("a thought\n");
        const { box, focused } = boxOn(m);
        box.start();
        await box.commit("notes.md");

        expect(h.moves).toEqual(["c:/repo/notes.md"]);
        expect(h.files.get("c:/repo/notes.md")).toBe("a thought\n");
        const tab = tabsOf(m).find((t) => t.id === id)!;
        expect(tab).toMatchObject({ filePath: "c:/repo/notes.md", dirty: false });
        expect(tab.isScratch).toBeFalsy();
        expect(box.tabId()).toBeNull();
        expect(focused.times).toBe(1);
    });

    it("with no file shown yet: the folder the last save went to, else the home folder", async () => {
        const first = mount();
        await first.openScratch(false);
        const a = boxOn(first).box;
        a.start();
        a.setTyped("one.md");
        expect(a.line()?.text).toBe("Saves to ~/one.md");
        await a.commit("c:/repo/docs/one.md");
        expect(tabsOf(first)[0].filePath).toBe("c:/repo/docs/one.md");

        const second = mount();
        await second.openScratch(false);
        const b = boxOn(second).box;
        b.start();
        b.setTyped("two.md");
        expect(b.line()?.text).toBe("Saves to c:/repo/docs/two.md");
    });

    it("an unchanged name closes the box and saves nothing", async () => {
        const m = mount();
        await m.openScratch(false);
        const { box } = boxOn(m);
        box.start();
        await box.commit("Untitled-1");
        expect(box.tabId()).toBeNull();
        expect(h.moves).toEqual([]);
        expect(tabsOf(m)[0].isScratch).toBe(true);
    });

    it("refused: the reason is under the box, not in the pane's error line, and the document stays Untitled with its text", async () => {
        const m = mount();
        await openLoaded(m, "c:/repo/a.ts");
        const id = (await m.openScratch(false))!;
        m.onContentChange("keep me\n");
        const { box } = boxOn(m);
        box.start();
        await box.commit("b.ts");
        expect(box.line()).toEqual({ kind: "error", text: "Something named “b.ts” already exists here." });
        h.failMove = "There isn't enough space to save it there.";
        await box.commit("notes.md");
        expect(box.line()?.text).toBe("There isn't enough space to save it there.");
        await box.commit("../notes.md");
        expect(box.line()?.text).toBe("A path can't contain “..”.");

        const tab = tabsOf(m).find((t) => t.id === id)!;
        expect(tab.isScratch).toBe(true);
        expect(tab.loadError ?? null).toBeNull();
        expect(m._contentByTab.get(id)).toBe("keep me\n");
        expect(h.files.get("c:/repo/b.ts")).toBe("const b = 2;\n");
    });
});
