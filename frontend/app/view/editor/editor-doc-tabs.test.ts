// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's files as document tabs (docs/specs/SPEC_DOCUMENT_TABS_2026_10_02.md
 * §6.1): kept in the block's `doctabs` record and restored from it, a restored
 * tab reading its file only when shown, a pane from before tabs opening its
 * `file`, and closing a tab with unsaved changes asking first.
 */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";

const h = vi.hoisted(() => ({
    files: new Map<string, string>(),
    reads: [] as string[],
    watched: [] as string[],
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
            return { content, read_only: false };
        }),
        WatchEditorFileCommand: vi.fn(async (_c: unknown, req: { path: string }) => {
            h.watched.push(req.path);
        }),
        UnwatchEditorFileCommand: vi.fn(async () => {}),
        CreateScratchFileCommand: vi.fn(async () => ({ file_path: "c:/scratch/s1.md", scratch_id: "s1", display_name: "Untitled-1" })),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/services", () => ({ WorkspaceService: {} }));
vi.mock("@/app/tab/tab-presets", () => ({ createBlockOnModel: () => {}, waitForLayoutModel: async () => null }));
vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: () => () => {} }));
vi.mock("@/app/drag/file-drop", () => ({ registerFileDropTarget: () => () => {} }));
vi.mock("@/app/drag/file-drop-actions", () => ({ notifyDrop: { cantOpen: () => {} } }));
vi.mock("@/app/util/reveal-block", () => ({ showBlockWithoutFocus: async () => {} }));

import { EditorViewModel } from "./editor-model";

// A fresh block per pane: the slice's event sink is installed once per
// module (editor-model.ts), so slots are never reset here.
let blocks = 0;
function mount(meta: Record<string, unknown> = {}) {
    const [m, setM] = createSignal<Record<string, unknown>>(meta);
    const ctx: PaneTabHostContext = {
        blockId: `ed${++blocks}`,
        meta: () => m() as MetaType,
        setMeta: async (patch) => {
            const next = { ...m() };
            for (const [k, v] of Object.entries(patch)) {
                if (v === null) delete next[k];
                else next[k] = v;
            }
            setM(next);
        },
        isFocused: () => true,
        visibility: () => "active",
    };
    const model = new EditorViewModel(ctx);
    return { model, meta: m };
}

const settle = () => new Promise((r) => setTimeout(r, 0));
async function until(check: () => boolean, ms = 1000): Promise<void> {
    const end = Date.now() + ms;
    while (!check()) {
        if (Date.now() > end) throw new Error("timed out");
        await new Promise((r) => setTimeout(r, 10));
    }
}

let model: EditorViewModel | null = null;

beforeEach(() => {
    h.files = new Map([
        ["c:/repo/a.ts", "const a = 1;\n"],
        ["c:/repo/b.md", "# b\n"],
        ["c:/repo/c.rs", "fn c() {}\n"],
    ]);
    h.reads = [];
    h.watched = [];
});

afterEach(() => {
    model?.dispose();
    model = null;
});

describe("the Editor's document tabs", () => {
    it("writes its tabs to the block, and a new pane restores them, reading only the tab in front", async () => {
        const first = mount();
        model = first.model;
        await model.openFile("C:\\repo\\a.ts");
        await model.openFile("C:\\repo\\b.md");
        await until(() => first.meta().doctabs != null && (first.meta().doctabs as { tabs: unknown[] }).tabs.length === 2);
        const record = first.meta().doctabs as { active: number; tabs: { key: string; state: { path: string } }[] };
        expect(record.active).toBe(1);
        expect(record.tabs.map((t) => t.state.path)).toEqual(["c:/repo/a.ts", "c:/repo/b.md"]);
        model.dispose();

        h.reads = [];
        model = mount({ doctabs: record }).model;
        expect(model.tabsAtom().map((t) => t.filePath)).toEqual(["c:/repo/a.ts", "c:/repo/b.md"]);
        expect(model.filePathAtom()).toBe("c:/repo/b.md");
        await until(() => model!.contentAtom() === "# b\n");
        expect(h.reads).toEqual(["c:/repo/b.md"]);
        // The other tab reads its file when it is shown.
        model.cycleTab(1);
        await until(() => model!.contentAtom() === "const a = 1;\n");
        expect(h.reads).toEqual(["c:/repo/b.md", "c:/repo/a.ts"]);
        expect(h.watched).toContain("c:/repo/a.ts");
    });

    it("a pane from before tabs opens its one file", async () => {
        const v = mount({ file: "c:/repo/c.rs" });
        model = v.model;
        await until(() => model!.contentAtom() === "fn c() {}\n");
        expect(model.tabsAtom().map((t) => t.filePath)).toEqual(["c:/repo/c.rs"]);
        await until(() => v.meta().doctabs != null);
    });

    it("closing every tab clears the record and the pre-tabs file", async () => {
        const v = mount({ file: "c:/repo/c.rs" });
        model = v.model;
        await until(() => model!.contentAtom() === "fn c() {}\n");
        await until(() => v.meta().doctabs != null);
        model.closeTab(model.activeIdAtom()!);
        await until(() => v.meta().doctabs === undefined);
        expect(v.meta().file).toBeUndefined();
    });

    it("asks before closing a tab with unsaved changes; discarding closes it", async () => {
        model = mount().model;
        await model.openFile("c:/repo/a.ts");
        await until(() => model!.contentAtom() !== "");
        model.onContentChange("const a = 2;\n");
        expect(model.dirtyAtom()).toBe(true);
        const asked = vi.fn();
        model.confirmDirtyClose = asked;
        model.closeTab(model.activeIdAtom()!);
        expect(model.tabsAtom()).toHaveLength(1);
        expect(asked).toHaveBeenCalledTimes(1);
        expect(asked.mock.calls[0][0].filePath).toBe("c:/repo/a.ts");
        asked.mock.calls[0][1]();
        expect(model.tabsAtom()).toHaveLength(0);
    });

    it("moves the tab in front, and reopens the last closed tab (Ctrl+Shift+T)", async () => {
        model = mount().model;
        await model.openFile("c:/repo/a.ts");
        await model.openFile("c:/repo/b.md");
        model.moveActiveTab(-1);
        expect(model.tabsAtom().map((t) => t.filePath)).toEqual(["c:/repo/b.md", "c:/repo/a.ts"]);
        model.closeTab(model.activeIdAtom()!);
        expect(model.tabsAtom().map((t) => t.filePath)).toEqual(["c:/repo/a.ts"]);
        model.reopenLastClosed();
        expect(model.filePathAtom()).toBe("c:/repo/b.md");
        await settle();
        await until(() => model!.contentAtom() === "# b\n");
    });
});
