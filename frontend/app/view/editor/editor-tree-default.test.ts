// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * An Editor opened on a file starts with its tree collapsed, whoever opened it; one with no file keeps the
 * tree. A value the pane holds wins, and the first decision is kept in the pane's meta, so the person's
 * toggle and a pane that began without a file both come back as they were.
 */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";

const h = vi.hoisted(() => ({
    files: new Map<string, string>(),
    reads: [] as string[],
    readRequests: [] as Record<string, unknown>[],
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
        // The probe that picks the viewer (SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS §4.2): every file here is text.
        FileProbeCommand: vi.fn(async (_c: unknown, req: { path: string }) => {
            const content = h.files.get(req.path);
            if (content === undefined) throw new Error(`no such file: ${req.path}`);
            return { size: content.length, mtime: 0, kind: "text", mime: "text/plain", text: { encoding: "utf-8", bom: "none" }, read_only: false };
        }),
        ReadEditorFileCommand: vi.fn(async (_c: unknown, req: { path: string }) => {
            h.reads.push(req.path);
            h.readRequests.push(req);
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
vi.mock("@/app/store/mps", () => {
    const mock = { muxEventSubscribe: () => () => {} };
    // As the real onMuxEvent: a typed subscription is a muxEventSubscribe whose handler gets the data.
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        (mock.muxEventSubscribe as (sub: unknown) => () => void)({ eventType, scope, handler: (e: any) => { if (e?.data != null) handler(e.data, e); } } as any);
    return { ...mock, onMuxEvent };
});
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
let models: EditorViewModel[] = [];
const open = (meta: Record<string, unknown> = {}) => {
    const v = mount(meta);
    models.push(v.model);
    return v;
};

beforeEach(() => {
    h.files = new Map([
        ["c:/repo/a.ts", "const a = 1;"],
        ["c:/repo/doc.pdf", "%PDF"],
        ["c:/repo/pic.png", "png"],
    ]);
    h.reads = [];
    h.readRequests = [];
    h.watched = [];
});

afterEach(() => {
    models.forEach((m) => m.dispose());
    models = [];
});

describe("the Editor's tree default", () => {
    it.each(["c:/repo/a.ts", "c:/repo/doc.pdf", "c:/repo/pic.png", "c:/repo/README.md"])("starts collapsed on a file: %s", async (file) => {
        const v = open({ file });
        expect(v.model.treeExpandedAtom()).toBe(false);
        await settle();
        expect(v.meta()["editor:tree_expanded"]).toBe(false); // the decision is kept
    });

    it("keeps the tree for an Editor with no file", async () => {
        const v = open({});
        expect(v.model.treeExpandedAtom()).toBe(true);
        await settle();
        expect(v.meta()["editor:tree_expanded"]).toBe(true);
    });

    it("lets an explicit value win either way", () => {
        expect(open({ file: "c:/repo/a.ts", "editor:tree_expanded": true }).model.treeExpandedAtom()).toBe(true);
        expect(open({ "editor:tree_expanded": false }).model.treeExpandedAtom()).toBe(false);
    });

    it("keeps the person's toggle: expanding a file's tree comes back expanded", async () => {
        const v = open({ file: "c:/repo/a.ts" });
        await settle();
        await v.model.toggleTreeExpanded();
        expect(v.model.treeExpandedAtom()).toBe(true);
        expect(v.meta()["editor:tree_expanded"]).toBe(true);
        expect(open(v.meta()).model.treeExpandedAtom()).toBe(true);
    });

    it("keeps an Editor that began without a file expanded when a file is opened in it later", async () => {
        const v = open({});
        await settle();
        await v.model.openFile("c:/repo/a.ts");
        expect(v.model.treeExpandedAtom()).toBe(true);
        // Loaded again, from the meta it now has (a `file`, and the tree decision made at the start).
        expect(open(v.meta()).model.treeExpandedAtom()).toBe(true);
    });
});
