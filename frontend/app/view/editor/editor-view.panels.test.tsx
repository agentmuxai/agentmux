// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the Editor draws around an open file, pinned as markup: the file tree column, the Markdown
 * mode toolbar and preview, the panel for a file that would not open, the language server's install
 * banner and status chip. The real view is mounted over a real EditorViewModel with a file open.
 * A refactor of the view must leave the committed snapshots as they are; regenerate with
 * `vitest -u` only for a change meant to look different.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";

const h = vi.hoisted(() => ({
    files: new Map<string, string>(),
    failRead: new Set<string>(),
    lsp: { mode: "missing" as "missing" | "crashed" | "starting" },
    clip: vi.fn(async (..._a: unknown[]) => {}),
    inflight: new Set<Promise<unknown>>(),
}));

vi.mock("@/app/store/global", async (orig) => ({
    ...(await orig<typeof import("@/app/store/global")>()),
    pushNotification: () => {},
    setActiveTab: () => {},
    settingsAtom: () => ({}),
}));
vi.mock("@/app/store/rpc-api", () => {
    const track = <T,>(p: Promise<T>): Promise<T> => {
        h.inflight.add(p);
        void p.then(
            () => h.inflight.delete(p),
            () => h.inflight.delete(p),
        );
        return p;
    };
    const specific: Record<string, unknown> = {
        GetEditorRootsCommand: async () => ({ home: "/home/u", drives: [] }),
        ListEditorDirCommand: async (_c: unknown, req: { path: string }) => ({
            path: req.path,
            entries: req.path === "/home/u" ? [{ name: "src", is_dir: true, is_symlink: false }, { name: "a.ts", is_dir: false, is_symlink: false }] : [],
        }),
        FileProbeCommand: async () => ({ size: 0, mtime: 0, kind: "text", mime: "text/plain", text: { encoding: "utf-8", bom: "none" }, read_only: false }),
        ReadEditorFileCommand: (_c: unknown, req: { path: string }) =>
            track(
                h.failRead.has(req.path)
                    ? Promise.reject(new Error("Permission denied (os error 13)"))
                    : Promise.resolve({ content: h.files.get(req.path) ?? "", mtime: 1, size: 1 }),
            ),
        LspStartCommand: () =>
            h.lsp.mode === "starting"
                ? // Still starting: never answers, and no test waits for it.
                  new Promise(() => {})
                : track(Promise.reject(new Error(h.lsp.mode === "missing" ? "server_binary_not_found" : "spawn failed"))),
    };
    return { RpcApi: new Proxy(specific, { get: (t, k) => (k in t ? t[k as string] : async () => ({})) }) };
});
vi.mock("@/util/clipboard", () => ({ writeText: (...a: unknown[]) => h.clip(...a) }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/services", () => ({ WorkspaceService: {} }));
vi.mock("@/app/tab/tab-presets", () => ({ createBlockOnModel: () => {}, waitForLayoutModel: async () => null }));
vi.mock("@/app/store/mps", () => {
    const mock = { muxEventSubscribe: () => () => {} };
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
        blockId: `panels${++blocks}`,
        meta: () => m() as MetaType,
        setMeta: async (patch) => void setM({ ...m(), ...patch }),
        isFocused: () => true,
        visibility: () => "active",
    };
    const model = new EditorViewModel(ctx);
    models.push(model);
    return model;
}

const flush = () => new Promise((r) => setTimeout(r, 0));
const settle = async () => {
    // The editor builds in steps (language pack, CodeMirror, then the server): give each its turn.
    for (let i = 0; i < 6; i++) {
        await new Promise((r) => setTimeout(r, 25));
        await Promise.allSettled([...h.inflight]);
    }
};

async function mountWith(path: string) {
    const model = modelOf();
    const view = render(() => <EditorViewComponent model={model} />);
    await settle();
    await model.openFile(path);
    await settle();
    return { model, ...view };
}

/** One element per line, so a snapshot diff points at the element that changed. The Markdown
 *  renderer's random heading ids are blanked. */
const markup = (...els: (Element | null)[]): string =>
    els
        .map((e) => (e ? e.outerHTML : "(absent)").replace(/></g, ">\n<").replace(/id="[0-9a-f-]{36}/g, 'id="<uuid>'))
        .join("\n----\n") + "\n";
const snap = (name: string) => `./__snapshots__/editor-view.panels.${name}.html`;
const q = (root: ParentNode, sel: string) => root.querySelector(sel);

beforeEach(() => {
    // jsdom has no layout: CodeMirror measures text through Range rects.
    const none = { length: 0, item: () => null, [Symbol.iterator]: function* () {} };
    Range.prototype.getClientRects = () => none as unknown as DOMRectList;
    Range.prototype.getBoundingClientRect = () => new DOMRect();
    h.files.clear();
    h.failRead.clear();
    h.lsp.mode = "missing";
    h.clip.mockClear();
    h.files.set("/home/u/a.ts", "export const a = 1;\n");
    h.files.set("/home/u/notes.md", "# Notes\n\nSome *text*.\n");
});
afterEach(() => {
    cleanup();
    for (const m of models.splice(0)) m.dispose();
});

describe("the file tree column", () => {
    it("is drawn beside the open file, with its resize handle", async () => {
        const v = await mountWith("/home/u/a.ts");
        await expect(markup(q(v.container, ".editor-tree-column"), q(v.container, ".editor-tree-resize-handle"))).toMatchFileSnapshot(snap("tree"));
    });

    it("gives way to a button that shows it again when hidden", async () => {
        const v = await mountWith("/home/u/a.ts");
        await v.model.toggleTreeExpanded();
        await settle();
        expect(q(v.container, ".editor-tree-column")).toBeNull();
        expect(q(v.container, ".editor-view")!.classList.contains("editor-view--tree-collapsed")).toBe(true);
        await expect(markup(q(v.container, ".editor-tree-toggle"))).toMatchFileSnapshot(snap("tree-toggle"));
    });
});

describe("a Markdown file", () => {
    it("offers Preview, Source and Split, and draws the preview in each mode", async () => {
        const v = await mountWith("/home/u/notes.md");
        await expect(markup(q(v.container, ".editor-mode-toolbar"), q(v.container, ".editor-preview-pane"), q(v.container, ".editor-codemirror"))).toMatchFileSnapshot(snap("md-preview"));
        v.model.setEditorMode("split");
        await settle();
        await expect(markup(q(v.container, ".editor-preview-divider"), q(v.container, ".editor-preview-pane"), q(v.container, ".editor-codemirror"))).toMatchFileSnapshot(snap("md-split"));
        v.model.setEditorMode("source");
        await settle();
        expect(q(v.container, ".editor-preview-pane")).toBeNull();
        expect(q(v.container, ".editor-preview-divider")).toBeNull();
        expect((q(v.container, ".editor-codemirror") as HTMLElement).style.display).toBe("");
    });

    it("has no mode toolbar for code", async () => {
        h.lsp.mode = "starting";
        const v = await mountWith("/home/u/a.ts");
        expect(q(v.container, ".editor-mode-toolbar")).toBeNull();
        expect(q(v.container, ".editor-preview-pane")).toBeNull();
    });
});

describe("a file that would not open", () => {
    it("fills the body with an error panel that closes the tab", async () => {
        h.failRead.add("/home/u/a.ts");
        const v = await mountWith("/home/u/a.ts");
        await expect(markup(q(v.container, ".editor-error-panel"))).toMatchFileSnapshot(snap("error-panel"));
        expect(q(v.container, ".editor-codemirror")).toBeNull();
        fireEvent.click(q(v.container, ".editor-error-panel-close")!);
        await settle();
        expect(q(v.container, ".editor-error-panel")).toBeNull();
    });
});

describe("the language server's install banner and status chip", () => {
    it("says a missing server is not installed, with the command to copy and its docs", async () => {
        const v = await mountWith("/home/u/a.ts");
        await expect(markup(q(v.container, ".editor-lsp-banner"), q(v.container, ".editor-lsp-status"))).toMatchFileSnapshot(snap("lsp-missing"));
        fireEvent.click([...v.container.querySelectorAll(".editor-lsp-banner button")].find((b) => b.textContent === "Copy")!);
        expect(h.clip).toHaveBeenCalledWith("npm install -g typescript-language-server typescript");
    });

    it("dismisses the banner for the session", async () => {
        const v = await mountWith("/home/u/a.ts");
        fireEvent.click(q(v.container, '.editor-lsp-banner [aria-label="Dismiss banner for this session"]')!);
        expect(q(v.container, ".editor-lsp-banner")).toBeNull();
    });

    it("shows a crashed server as a chip, with no banner", async () => {
        h.lsp.mode = "crashed";
        const v = await mountWith("/home/u/a.ts");
        expect(q(v.container, ".editor-lsp-banner")).toBeNull();
        await expect(markup(q(v.container, ".editor-lsp-status"))).toMatchFileSnapshot(snap("lsp-crashed"));
    });

    it("shows a server still starting as a chip", async () => {
        h.lsp.mode = "starting";
        const v = await mountWith("/home/u/a.ts");
        await expect(markup(q(v.container, ".editor-lsp-banner"), q(v.container, ".editor-lsp-status"))).toMatchFileSnapshot(snap("lsp-starting"));
    });
});
