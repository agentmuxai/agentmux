// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the Editor view does around CodeMirror, pinned as ordered logs: how a view is built and
 * rebuilt, how a tab's state is kept across switches and dropped on close, the language server
 * client's lifecycle, the pane's commands and keys, live reconfiguration (word wrap, theme, save
 * keys), the resize handles and the clean-up on unmount. The real view runs over a real
 * EditorViewModel and real CodeMirror; only the language server client is a recording stand-in.
 * A refactor of the view must leave the committed snapshots as they are; regenerate with
 * `vitest -u` only for a change meant to behave differently.
 *
 * An edit never rebuilds the view: the first one flips the tab dirty, and the tab-change effect
 * depends on the tab's identity, its loading state, its content and its viewer, not on the tab
 * object (#3260, #5016). "editing the open tab" pins that; the other logs show a rebuild only where
 * a tab or its file really changed.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";

const h = vi.hoisted(() => ({
    log: [] as string[],
    files: new Map<string, string>(),
    lsp: { startOk: true, n: 0 },
    settings: {} as Record<string, unknown>,
    runners: new Map<string, (command: string) => boolean>(),
    readOnly: new Set<string>(),
    listing: [] as { name: string; is_dir: boolean; is_symlink: boolean }[],
    inflight: new Set<Promise<unknown>>(),
}));

vi.mock("@/app/store/global", async (orig) => ({
    ...(await orig<typeof import("@/app/store/global")>()),
    pushNotification: () => {},
    setActiveTab: () => {},
    settingsAtom: () => h.settings,
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
    const specific: Record<string, (c: unknown, req: any) => unknown> = {
        GetEditorRootsCommand: async () => ({ home: "/home/u", drives: [] }),
        ListEditorDirCommand: async (_c, req) => ({ path: req.path, entries: req.path === "/home/u" ? h.listing : [] }),
        FileProbeCommand: async (_c, req) => ({ size: 0, mtime: 0, kind: "text", mime: "text/plain", text: { encoding: "utf-8", bom: "none" }, read_only: h.readOnly.has(req.path) }),
        ReadEditorFileCommand: (_c, req) => track(Promise.resolve({ content: h.files.get(req.path) ?? "", mtime: 1, size: 1, read_only: h.readOnly.has(req.path) })),
        CreateScratchFileCommand: async () => ({ file_path: "/tmp/scratch-1.md", scratch_id: "scratch-1", display_name: "Untitled-1" }),
    };
    const quiet = new Set(["GetEditorRootsCommand", "ListEditorDirCommand", "FileProbeCommand"]);
    return {
        RpcApi: new Proxy(specific, {
            get: (t, k: string) => async (c: unknown, req: any) => {
                if (!quiet.has(k)) h.log.push(`rpc ${k.replace(/Command$/, "")} ${JSON.stringify(req ?? null)}`);
                return k in t ? t[k](c, req) : {};
            },
        }),
    };
});
vi.mock("@/util/clipboard", () => ({ writeText: async () => {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/services", () => ({ WorkspaceService: {} }));
vi.mock("@/app/tab/tab-presets", () => ({ createBlockOnModel: () => {}, waitForLayoutModel: async () => null }));
vi.mock("@/app/store/mps", () => {
    const mock = { muxEventSubscribe: () => () => {} };
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        (mock.muxEventSubscribe as (sub: unknown) => () => void)({ eventType, scope, handler: (e: any) => { if (e?.data != null) handler(e.data, e); } } as any);
    return { ...mock, onMuxEvent };
});
vi.mock("./open-from-remote-modal", () => ({
    OpenFromRemoteModal: (p: { current: string; onOpen: (h: string, path: string) => void; onClose: () => void }) => (
        <div class="stub-remote" data-current={p.current}>
            <button class="stub-remote-open" onClick={() => p.onOpen("host1", "/srv/x.ts")} />
            <button class="stub-remote-close" onClick={() => p.onClose()} />
        </div>
    ),
}));
vi.mock("@/app/drag/file-drop", () => ({ registerFileDropTarget: () => () => {} }));
vi.mock("@/app/drag/file-drop-actions", () => ({ notifyDrop: { cantOpen: () => {}, cantMove: () => {} } }));
vi.mock("@/app/util/reveal-block", () => ({ showBlockWithoutFocus: async () => {} }));
vi.mock("@/app/keybindings/app-api", async (orig) => {
    const real = await orig<typeof import("@/app/keybindings/app-api")>();
    return {
        ...real,
        noteResolved: (command: string, scope: string) => h.log.push(`resolved ${command} ${scope}`),
        registerPaneCommandRunner: (blockId: string, run: (c: string) => boolean, refuse?: unknown) => {
            h.runners.set(blockId, run);
            const off = real.registerPaneCommandRunner(blockId, run, refuse as never);
            return () => {
                h.log.push("pane command runner removed");
                off();
            };
        },
    };
});
vi.mock("./lsp/lsp-client", () => {
    class FakeLsp {
        id = ++h.lsp.n;
        state: any = { kind: "starting" };
        cb: ((s: unknown) => void) | undefined;
        opened: string | null = null;
        constructor(public language: string, public filePath: string) {
            h.log.push(`lsp#${this.id} new ${language} ${filePath}`);
        }
        onStateChange(cb: (s: unknown) => void) {
            this.cb = cb;
            return () => h.log.push(`lsp#${this.id} state unsubscribed`);
        }
        async start() {
            h.log.push(`lsp#${this.id} start`);
            this.state = h.lsp.startOk ? { kind: "ready" } : { kind: "missing", language: this.language, installCommand: "install it", docsUrl: "" };
            this.cb?.(this.state);
            return h.lsp.startOk;
        }
        getState() {
            return this.state;
        }
        getWorkspaceRoot() {
            return "/home/u";
        }
        getOpenedFileUri() {
            return this.opened;
        }
        async didOpen(path: string, content: string, language: string) {
            h.log.push(`lsp#${this.id} didOpen ${path} ${language} ${JSON.stringify(content)}`);
            this.opened = `file://${path}`;
        }
        async didChange(path: string, content: string) {
            h.log.push(`lsp#${this.id} didChange ${path} ${JSON.stringify(content)}`);
        }
        async didClose(path: string) {
            h.log.push(`lsp#${this.id} didClose ${path}`);
        }
        async dispose() {
            h.log.push(`lsp#${this.id} dispose`);
        }
    }
    return { LspClient: FakeLsp };
});
vi.mock("./lsp/lsp-extensions", () => ({
    lspDiagnosticsExtension: (_view: unknown, client: { id: number }) => {
        h.log.push(`diagnostics wired to lsp#${client.id}`);
        return [[], () => h.log.push(`diagnostics of lsp#${client.id} unwired`)];
    },
}));

import { EditorViewComponent } from "./editor-view";
import { EditorViewModel } from "./editor-model";
import { undo } from "@codemirror/commands";
import { language } from "@codemirror/language";
import { EditorState } from "@codemirror/state";
import { setUserKeybindings } from "@/app/keybindings/registry";

let blocks = 0;
const models: EditorViewModel[] = [];
function modelOf(): EditorViewModel {
    const [m, setM] = createSignal<Record<string, unknown>>({});
    const ctx: PaneTabHostContext = {
        blockId: `session${++blocks}`,
        meta: () => m() as MetaType,
        setMeta: async (patch) => void setM({ ...m(), ...patch }),
        isFocused: () => true,
        visibility: () => "active",
    };
    const model = new EditorViewModel(ctx);
    models.push(model);
    return model;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const settle = async () => {
    for (let i = 0; i < 6; i++) {
        await sleep(25);
        await Promise.allSettled([...h.inflight]);
    }
};

async function mountView() {
    const model = modelOf();
    const view = render(() => <EditorViewComponent model={model} />);
    await settle();
    return { model, ...view };
}
async function open(model: EditorViewModel, path: string): Promise<string> {
    await model.openFile(path);
    await settle();
    return model.activeIdAtom()!;
}

const cm = (model: EditorViewModel) => model.cmViewRef.current;
/** The editor as the person sees it: text, caret, and how many CodeMirror views hang in the pane. */
const probe = (v: { model: EditorViewModel; container: HTMLElement }, label: string) => {
    const view = cm(v.model);
    h.log.push(
        `probe ${label}: doc=${JSON.stringify(view?.state.doc.toString() ?? null)} caret=${view?.state.selection.main.head ?? null}` +
            ` dirty=${v.model.dirtyAtom()} views=${v.container.querySelectorAll(".cm-editor").length} tab=${v.model.activeTabAtom()?.displayName ?? null}`,
    );
};
/** The log with the generated ids (block counter, tab ids) blanked, so it does not depend on test order or time. */
const out = () => h.log.join("\n").replace(/session\d+/g, "session<n>").replace(/dt-[a-z0-9]+-[a-z0-9]+/g, "dt-<id>") + "\n";
const snap = (name: string) => `./__snapshots__/editor-view.session.${name}.txt`;
/** An edit; wait for it and its debounced language-server change to settle. */
const type = async (model: EditorViewModel, at: number, text: string) => {
    cm(model)!.dispatch({ changes: { from: at, insert: text }, selection: { anchor: at + text.length } });
    await settle();
};
const moveCaret = (model: EditorViewModel, to: number) => cm(model)!.dispatch({ selection: { anchor: to } });
const run = (model: EditorViewModel, command: string): boolean => {
    const ran = h.runners.get(model.blockId)!(command);
    h.log.push(`run ${command} -> ${ran}`);
    return ran;
};

beforeEach(() => {
    const none = { length: 0, item: () => null, [Symbol.iterator]: function* () {} };
    Range.prototype.getClientRects = () => none as unknown as DOMRectList;
    Range.prototype.getBoundingClientRect = () => new DOMRect();
    h.log.length = 0;
    h.files.clear();
    h.runners.clear();
    h.settings = {};
    h.readOnly.clear();
    h.listing = [{ name: "src", is_dir: true, is_symlink: false }, { name: "a.ts", is_dir: false, is_symlink: false }];
    h.lsp.startOk = true;
    h.lsp.n = 0;
    h.files.set("/home/u/a.ts", "export const a = 1;\n");
    h.files.set("/home/u/b.ts", "export const b = 2;\n");
    h.files.set("/home/u/c.py", "c = 3\n");
    h.files.set("/home/u/notes.md", "# Notes\n\nSome *text*.\n");
    h.files.set("/home/u/plain.txt", "plain text\n");
});
afterEach(async () => {
    cleanup();
    for (const m of models.splice(0)) m.dispose();
    await settle();
    document.documentElement.removeAttribute("data-theme-polarity");
});

describe("building the view", () => {
    it("builds one CodeMirror for an opened file, reads it once and takes the keyboard", async () => {
        const v = await mountView();
        probe(v, "empty");
        await open(v.model, "/home/u/plain.txt");
        probe(v, "opened");
        h.log.push(`body has focus in editor: ${document.activeElement === cm(v.model)?.contentDOM}`);
        await expect(out()).toMatchFileSnapshot(snap("build-one-view"));
    });

    it("two opens in a row leave one view, with the second file", async () => {
        const v = await mountView();
        void v.model.openFile("/home/u/a.ts");
        void v.model.openFile("/home/u/b.ts");
        await settle();
        probe(v, "after two opens");
        // The two watch requests go out in whichever order the reads finish: not pinned.
        await expect(out().split("\n").filter((l) => !l.startsWith("rpc WatchEditorFile")).join("\n")).toMatchFileSnapshot(snap("build-two-opens"));
    });

    it("a pane that mounts with a file already open builds its view at once", async () => {
        const model = modelOf();
        await model.openFile("/home/u/plain.txt");
        const view = render(() => <EditorViewComponent model={model} />);
        await settle();
        probe({ model, container: view.container }, "mounted with file");
        await expect(out()).toMatchFileSnapshot(snap("build-mounted-with-file"));
    });

    it("opens at a line when the open asked for one", async () => {
        const v = await mountView();
        await v.model.openFile("/home/u/a.ts", { line: 1 } as never);
        await settle();
        probe(v, "at line 1");
        await expect(out()).toMatchFileSnapshot(snap("build-at-line"));
    });
});

describe("switching and closing tabs", () => {
    it("keeps each tab's caret across switches, and its text and undo history after an edit", async () => {
        const v = await mountView();
        const a = await open(v.model, "/home/u/a.ts");
        moveCaret(v.model, 7);
        const b = await open(v.model, "/home/u/b.ts");
        moveCaret(v.model, 3);
        probe(v, "b, caret set");
        const to = async (id: string, label: string) => {
            v.model.switchTab(id);
            await settle();
            probe(v, label);
        };
        await to(a, "a: caret kept");
        await to(b, "b: caret kept");
        await to(a, "a again");
        await type(v.model, 0, "// edit\n");
        probe(v, "a edited");
        await to(b, "b");
        await to(a, "back on a");
        undo(cm(v.model)!);
        probe(v, "a after undo");
        await expect(out()).toMatchFileSnapshot(snap("tabs-keep-state"));
    });

    it("rebuilds a clean tab whose file changed underneath it", async () => {
        const v = await mountView();
        const a = await open(v.model, "/home/u/a.ts");
        await open(v.model, "/home/u/b.ts");
        h.files.set("/home/u/a.ts", "export const a = 100;\n");
        (v.model as any)._contentByTab.set(a, "export const a = 100;\n");
        v.model.switchTab(a);
        await settle();
        probe(v, "a with new text");
        await expect(out()).toMatchFileSnapshot(snap("tabs-stale-snapshot"));
    });

    it("closing a tab drops its saved state, so reopening starts fresh", async () => {
        const v = await mountView();
        const a = await open(v.model, "/home/u/a.ts");
        await type(v.model, 0, "// edit\n");
        await v.model.saveFile();
        await open(v.model, "/home/u/b.ts");
        v.model.closeTab(a);
        await settle();
        probe(v, "a closed");
        await open(v.model, "/home/u/a.ts");
        probe(v, "a reopened");
        await expect(out()).toMatchFileSnapshot(snap("tabs-close-drops-state"));
    });

    it("closing a tab with unsaved changes asks first, and keeping it leaves it open", async () => {
        const v = await mountView();
        const a = await open(v.model, "/home/u/a.ts");
        await open(v.model, "/home/u/b.ts");
        v.model.switchTab(a);
        await settle();
        await type(v.model, 0, "x");
        v.model.closeTab(a);
        await settle();
        h.log.push(`dialog: ${document.body.querySelector(".confirm-dialog, [role=dialog], [role=alertdialog]")?.textContent ?? "(none)"}`);
        const buttons = [...document.body.querySelectorAll("button")].map((b) => b.textContent);
        h.log.push(`buttons: ${JSON.stringify(buttons)}`);
        fireEvent.click([...document.body.querySelectorAll("button")].find((b) => /cancel/i.test(b.textContent ?? ""))!);
        await settle();
        probe(v, "after cancel");
        v.model.closeTab(a);
        await settle();
        fireEvent.click([...document.body.querySelectorAll("button")].find((b) => /discard/i.test(b.textContent ?? ""))!);
        await settle();
        probe(v, "after discard");
        await expect(out()).toMatchFileSnapshot(snap("tabs-dirty-close"));
    });

    it("a tab in another viewer leaves CodeMirror hidden, and coming back restores the text", async () => {
        const v = await mountView();
        const a = await open(v.model, "/home/u/a.ts");
        await type(v.model, 0, "// edit\n");
        const t = v.container.querySelector(".editor-codemirror") as HTMLElement;
        const b = await open(v.model, "/home/u/b.ts");
        v.model.reopenWith(b, "image" as never);
        await settle();
        h.log.push(`codemirror display with a non-text viewer: ${JSON.stringify(t.style.display)}`);
        v.model.switchTab(a);
        await settle();
        probe(v, "back on a");
        await expect(out()).toMatchFileSnapshot(snap("tabs-other-viewer"));
    });
});

describe("editing the open tab", () => {
    const docOf = (model: EditorViewModel) => cm(model)?.state.doc.toString();
    /** The language server traffic since `from`: a rebuilt view closes and reopens the file. */
    const lspSince = (from: number) => h.log.slice(from).filter((l) => / (didOpen|didClose) /.test(l));

    it("the first edit keeps the typed text in the same view, without reopening the file for the language server (#3260, #5016)", async () => {
        const v = await mountView();
        await open(v.model, "/home/u/a.ts");
        const before = cm(v.model);
        const mark = h.log.length;
        await type(v.model, 0, "x");
        expect(v.model.dirtyAtom()).toBe(true);
        expect(cm(v.model)).toBe(before);
        expect(docOf(v.model)).toBe("xexport const a = 1;\n");
        expect(v.container.querySelectorAll(".cm-editor").length).toBe(1);
        expect(lspSince(mark)).toEqual([]);
        await type(v.model, 1, "y");
        expect(cm(v.model)).toBe(before);
        expect(docOf(v.model)).toBe("xyexport const a = 1;\n");
    });

    it("saving keeps the view and the text too, as the tab turns clean again", async () => {
        const v = await mountView();
        await open(v.model, "/home/u/a.ts");
        await type(v.model, 0, "x");
        const before = cm(v.model);
        const mark = h.log.length;
        await v.model.saveFile();
        await settle();
        expect(v.model.dirtyAtom()).toBe(false);
        expect(cm(v.model)).toBe(before);
        expect(docOf(v.model)).toBe("xexport const a = 1;\n");
        expect(lspSince(mark)).toEqual([]);
    });

    it("a clean tab whose file changed on disk is rebuilt with the new text", async () => {
        const v = await mountView();
        await open(v.model, "/home/u/a.ts");
        const before = cm(v.model);
        h.files.set("/home/u/a.ts", "export const a = 100;\n");
        await (v.model as any)._handleExternalFileChanged("/home/u/a.ts");
        await settle();
        expect(docOf(v.model)).toBe("export const a = 100;\n");
        expect(cm(v.model)).not.toBe(before);
        expect(v.container.querySelectorAll(".cm-editor").length).toBe(1);
    });

    it("a tab with unsaved changes keeps its text when its file changes on disk", async () => {
        const v = await mountView();
        await open(v.model, "/home/u/a.ts");
        await type(v.model, 0, "x");
        const before = cm(v.model);
        h.files.set("/home/u/a.ts", "export const a = 100;\n");
        await (v.model as any)._handleExternalFileChanged("/home/u/a.ts");
        await settle();
        expect(cm(v.model)).toBe(before);
        expect(docOf(v.model)).toBe("xexport const a = 1;\n");
        expect(v.model.dirtyAtom()).toBe(true);
    });

    it("switching to another tab and back rebuilds the view for the other file and restores this one's edit", async () => {
        const v = await mountView();
        const a = await open(v.model, "/home/u/a.ts");
        await type(v.model, 0, "x");
        await open(v.model, "/home/u/b.ts");
        expect(docOf(v.model)).toBe("export const b = 2;\n");
        v.model.switchTab(a);
        await settle();
        expect(docOf(v.model)).toBe("xexport const a = 1;\n");
        expect(v.container.querySelectorAll(".cm-editor").length).toBe(1);
    });
});

describe("the language server client", () => {
    it("starts for a supported file, sends one debounced change, and is reused for the next file of the language", async () => {
        const v = await mountView();
        await open(v.model, "/home/u/a.ts");
        h.log.push("-- two edits close together");
        await type(v.model, 0, "0");
        const view = cm(v.model)!;
        view.dispatch({ changes: { from: 1, insert: "1" } });
        view.dispatch({ changes: { from: 2, insert: "2" } });
        await sleep(350);
        h.log.push("-- next file, same language");
        await open(v.model, "/home/u/b.ts");
        h.log.push("-- a stale change must not follow");
        await sleep(350);
        await expect(out()).toMatchFileSnapshot(snap("lsp-reuse"));
    });

    it("is replaced by a language change, and dropped for an unsupported file", async () => {
        const v = await mountView();
        await open(v.model, "/home/u/a.ts");
        h.log.push("-- python");
        await open(v.model, "/home/u/c.py");
        h.log.push("-- markdown: unsupported");
        await open(v.model, "/home/u/notes.md");
        h.log.push(`status chip: ${v.container.querySelector(".editor-lsp-status")?.textContent ?? "(none)"}`);
        await expect(out()).toMatchFileSnapshot(snap("lsp-replace-and-drop"));
    });

    it("is not started when the setting is off, and a server that will not start gets no file", async () => {
        h.settings = { "editor:lsp.enabled": false };
        const v = await mountView();
        await open(v.model, "/home/u/a.ts");
        h.log.push("-- setting on, server missing");
        h.settings = {};
        h.lsp.startOk = false;
        await open(v.model, "/home/u/b.ts");
        h.log.push(`banner: ${v.container.querySelector(".editor-lsp-banner")?.textContent ?? "(none)"}`);
        await expect(out()).toMatchFileSnapshot(snap("lsp-off-and-failing"));
    });

    it("re-wires the open file when a tab is switched back to, and is disposed with the view", async () => {
        const v = await mountView();
        const a = await open(v.model, "/home/u/a.ts");
        await open(v.model, "/home/u/b.ts");
        h.log.push("-- back to a");
        v.model.switchTab(a);
        await settle();
        h.log.push("-- unmount");
        v.unmount();
        await settle();
        await expect(out()).toMatchFileSnapshot(snap("lsp-switch-back-and-dispose"));
    });
});

describe("the pane's commands and keys", () => {
    it("runs find, the preview toggle, save and focus-tree, and refuses what does not apply", async () => {
        const v = await mountView();
        run(v.model, "editor:find");
        await open(v.model, "/home/u/a.ts");
        run(v.model, "editor:togglePreview");
        run(v.model, "editor:find");
        h.log.push(`search panel open: ${v.container.querySelector(".cm-search") != null}`);
        await type(v.model, 0, "// edit\n");
        run(v.model, "editor:save");
        await settle();
        probe(v, "after save");
        run(v.model, "editor:saveAs");
        run(v.model, "editor:nonsense");
        await open(v.model, "/home/u/notes.md");
        run(v.model, "editor:togglePreview");
        h.log.push(`mode after toggle: ${v.model.editorMode()}`);
        run(v.model, "editor:find");
        await expect(out()).toMatchFileSnapshot(snap("commands-basic"));
    });

    it("moves the keyboard between the tree and the body, showing a hidden tree", async () => {
        const v = await mountView();
        await open(v.model, "/home/u/a.ts");
        const root = v.container.querySelector(".editor-view")!;
        h.log.push(`tree item focused first: ${JSON.stringify((document.activeElement as HTMLElement)?.className)}`);
        run(v.model, "editor:focusTree");
        fireEvent.focusIn(document.activeElement ?? root);
        h.log.push(`active: ${(document.activeElement as HTMLElement)?.getAttribute("role")} region class: ${v.container.querySelector(".editor-region--focused")?.className}`);
        run(v.model, "editor:focusTree");
        h.log.push(`active after second: in editor=${document.activeElement === cm(v.model)?.contentDOM}`);
        await v.model.toggleTreeExpanded();
        await settle();
        run(v.model, "editor:focusTree");
        await settle();
        h.log.push(`tree shown again: ${v.model.treeExpandedAtom()}`);
        await expect(out()).toMatchFileSnapshot(snap("commands-focus-tree"));
    });

    it("Save As on a scratch tab asks for a path in the tab, and confirming saves there", async () => {
        const v = await mountView();
        await v.model.openScratch();
        await settle();
        run(v.model, "editor:saveAs");
        await settle();
        h.log.push(`save-as input shown: ${v.container.querySelector(".editor-tab--saveas input") != null}`);
        const input = v.container.querySelector<HTMLInputElement>(".editor-tab--saveas input");
        if (input) {
            fireEvent.input(input, { target: { value: "/home/u/new.txt" } });
            fireEvent.keyDown(input, { key: "Enter" });
        }
        await settle();
        run(v.model, "editor:save");
        await expect(out()).toMatchFileSnapshot(snap("commands-save-as"));
    });

    it("answers the editor's keys before CodeMirror and leaves text boxes their own", async () => {
        const v = await mountView();
        await open(v.model, "/home/u/notes.md");
        const root = v.container.querySelector<HTMLElement>(".editor-view")!;
        const key = (init: KeyboardEventInit, target: Element = cm(v.model)!.contentDOM) => {
            const ev = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
            target.dispatchEvent(ev);
            h.log.push(`key ${JSON.stringify(init)} -> prevented=${ev.defaultPrevented}`);
        };
        for (const mod of [{ ctrlKey: true }, { metaKey: true }]) {
            key({ key: "f", code: "KeyF", ...mod });
            key({ key: "V", code: "KeyV", shiftKey: true, ...mod });
        }
        h.log.push(`mode: ${v.model.editorMode()}`);
        key({ key: "Tab", code: "Tab", ctrlKey: true });
        const input = document.createElement("input");
        root.appendChild(input);
        key({ key: "Tab", code: "Tab", ctrlKey: true }, input);
        input.remove();
        h.log.push(`active tab: ${v.model.activeTabAtom()?.displayName}`);
        await expect(out()).toMatchFileSnapshot(snap("keys-capture"));
    });

    it("saves from CodeMirror's own save key", async () => {
        const v = await mountView();
        await open(v.model, "/home/u/a.ts");
        await type(v.model, 0, "// edit\n");
        for (const mod of [{ ctrlKey: true }, { metaKey: true }]) {
            cm(v.model)!.contentDOM.dispatchEvent(new KeyboardEvent("keydown", { key: "s", code: "KeyS", bubbles: true, cancelable: true, ...mod }));
            await settle();
        }
        await expect(out()).toMatchFileSnapshot(snap("keys-save"));
    });
});

describe("live reconfiguration", () => {
    it("word wrap follows the setting live and is resynced on a tab switch", async () => {
        const v = await mountView();
        const a = await open(v.model, "/home/u/a.ts");
        const wrapped = () => v.container.querySelector(".cm-content")?.classList.contains("cm-lineWrapping");
        h.log.push(`wrapped initially: ${wrapped()}`);
        await v.model.toggleWordWrap();
        await settle();
        h.log.push(`wrapped after toggle: ${wrapped()}`);
        await open(v.model, "/home/u/b.ts");
        await v.model.toggleWordWrap();
        await settle();
        v.model.switchTab(a);
        await settle();
        h.log.push(`wrapped on a after a switch with it off: ${wrapped()}`);
        await expect(out()).toMatchFileSnapshot(snap("live-word-wrap"));
    });

    it("follows the theme's polarity, and stops when the view goes", async () => {
        const v = await mountView();
        await open(v.model, "/home/u/a.ts");
        const view = cm(v.model)!;
        const spy = vi.spyOn(view, "dispatch");
        document.documentElement.setAttribute("data-theme-polarity", "light");
        await sleep(10);
        h.log.push(`dispatches after a polarity change: ${spy.mock.calls.length}`);
        v.unmount();
        const after = spy.mock.calls.length;
        document.documentElement.setAttribute("data-theme-polarity", "dark");
        await sleep(10);
        h.log.push(`dispatches after unmount: ${spy.mock.calls.length - after}`);
        await expect(out()).toMatchFileSnapshot(snap("live-theme"));
    });

    it("zoom is CSS zoom on the pane", async () => {
        const v = await mountView();
        const root = v.container.querySelector<HTMLElement>(".editor-view")!;
        h.log.push(`zoom: ${JSON.stringify(root.style.zoom)} scope=${root.dataset.zoomScope}`);
        await expect(out()).toMatchFileSnapshot(snap("live-zoom"));
    });
});

describe("the resize handles", () => {
    it("drags the tree width live and saves it once on release", async () => {
        const v = await mountView();
        const commit = vi.spyOn(v.model, "commitTreeWidth").mockResolvedValue(undefined as never);
        const handle = v.container.querySelector(".editor-tree-resize-handle")!;
        const width0 = v.model.treeWidthAtom();
        fireEvent.mouseDown(handle, { clientX: 300 });
        h.log.push(`cursor while dragging: ${document.body.style.cursor} select=${document.body.style.userSelect}`);
        fireEvent.mouseMove(document, { clientX: 340 });
        h.log.push(`width change: ${v.model.treeWidthAtom() - width0}`);
        fireEvent.mouseUp(document);
        h.log.push(`cursor after: ${JSON.stringify(document.body.style.cursor)} select=${JSON.stringify(document.body.style.userSelect)} commits=${commit.mock.calls.length}`);
        fireEvent.mouseMove(document, { clientX: 500 });
        h.log.push(`width change after release: ${v.model.treeWidthAtom() - width0}`);
        await expect(out()).toMatchFileSnapshot(snap("resize-tree"));
    });

    it("drags the preview's height, up for taller", async () => {
        const v = await mountView();
        await open(v.model, "/home/u/notes.md");
        v.model.setEditorMode("split");
        await settle();
        const commit = vi.spyOn(v.model, "commitPreviewHeight").mockResolvedValue(undefined as never);
        const divider = v.container.querySelector(".editor-preview-divider")!;
        const h0 = v.model.previewHeightAtom();
        fireEvent.mouseDown(divider, { clientY: 400 });
        h.log.push(`cursor while dragging: ${document.body.style.cursor}`);
        fireEvent.mouseMove(document, { clientY: 360 });
        h.log.push(`height change: ${v.model.previewHeightAtom() - h0}`);
        fireEvent.mouseUp(document);
        h.log.push(`commits=${commit.mock.calls.length} cursor=${JSON.stringify(document.body.style.cursor)}`);
        await expect(out()).toMatchFileSnapshot(snap("resize-preview"));
    });
});

describe("what the view builds for each kind of file", () => {
    it("loads the language pack for each language, and none for plain text", async () => {
        for (const [file, text] of [["x.rs", "fn main() {}\n"], ["x.html", "<p>hi</p>\n"], ["x.css", "a { b: c }\n"], ["x.json", "{}\n"], ["x.js", "let x = 1;\n"], ["x.py", "x = 1\n"], ["x.md", "# t\n"], ["x.txt", "plain\n"]]) {
            h.files.set(`/home/u/${file}`, text);
        }
        const v = await mountView();
        for (const file of ["x.rs", "x.html", "x.css", "x.json", "x.js", "x.py", "x.md", "x.txt"]) {
            await open(v.model, `/home/u/${file}`);
            h.log.push(`${file}: language=${cm(v.model)?.state.facet(language)?.name ?? "(none)"}`);
        }
        await expect(out()).toMatchFileSnapshot(snap("build-languages"));
    });

    it("builds a read-only file as read-only", async () => {
        h.readOnly.add("/home/u/plain.txt");
        const v = await mountView();
        await open(v.model, "/home/u/plain.txt");
        h.log.push(`read-only: ${cm(v.model)?.state.facet(EditorState.readOnly)}`);
        await expect(out()).toMatchFileSnapshot(snap("build-read-only"));
    });

    it("follows a change of the user's keybindings without a rebuild", async () => {
        const v = await mountView();
        await open(v.model, "/home/u/a.ts");
        const spy = vi.spyOn(cm(v.model)!, "dispatch");
        setUserKeybindings([{ command: "editor:save", key: "ctrl+alt+s" }], (id) => id.startsWith("editor:"));
        h.log.push(`dispatches after the change: ${spy.mock.calls.length}`);
        setUserKeybindings([], () => false);
        h.log.push(`dispatches after resetting: ${spy.mock.calls.length}`);
        await expect(out()).toMatchFileSnapshot(snap("live-keybindings"));
    });
});

describe("where the keyboard goes", () => {
    it("lands in the text, in the viewer or in the pane, depending on what is shown", async () => {
        const v = await mountView();
        const where = () => {
            const a = document.activeElement as HTMLElement | null;
            return a === cm(v.model)?.contentDOM ? "codemirror" : (a?.className || a?.tagName || "(none)");
        };
        await open(v.model, "/home/u/a.ts");
        const tree = v.container.querySelector<HTMLElement>('.file-tree [role="treeitem"]');
        tree?.focus();
        fireEvent.focusIn(tree!);
        run(v.model, "editor:focusTree");
        h.log.push(`from the tree, text: ${where()}`);
        await open(v.model, "/home/u/notes.md");
        v.model.setEditorMode("preview");
        await settle();
        tree?.focus();
        fireEvent.focusIn(tree!);
        run(v.model, "editor:focusTree");
        h.log.push(`from the tree, markdown preview: ${where()}`);
        v.model.setEditorMode("source");
        await settle();
        tree?.focus();
        fireEvent.focusIn(tree!);
        run(v.model, "editor:focusTree");
        h.log.push(`from the tree, markdown source: ${where()}`);
        await expect(out()).toMatchFileSnapshot(snap("focus-body"));
    });
});

describe("the tree, the tab bar and the dialogs", () => {
    it("opens a file as a preview on a click and for good on a double click, and hides the tree from its toolbar", async () => {
        const v = await mountView();
        const preview = vi.spyOn(v.model, "openFilePreview").mockResolvedValue(undefined as never);
        const full = vi.spyOn(v.model, "openFile").mockResolvedValue(undefined as never);
        const row = screen.getByRole("treeitem", { name: "a.ts" }).querySelector(".file-tree-row")!;
        fireEvent.click(row);
        fireEvent.doubleClick(row);
        h.log.push(`preview=${JSON.stringify(preview.mock.calls)} open=${JSON.stringify(full.mock.calls)}`);
        const toolbar = [...v.container.querySelectorAll(".file-tree-toolbar button, .editor-tree-column button")].map((b) => b.getAttribute("aria-label") ?? b.textContent);
        h.log.push(`toolbar buttons: ${JSON.stringify(toolbar)}`);
        const hide = v.container.querySelector<HTMLElement>('.editor-tree-column button[aria-label="Hide file tree"]');
        if (hide) fireEvent.click(hide);
        await settle();
        h.log.push(`tree expanded after hide: ${v.model.treeExpandedAtom()}`);
        fireEvent.click(v.container.querySelector(".editor-tree-toggle")!);
        await settle();
        h.log.push(`tree expanded after the bar's button: ${v.model.treeExpandedAtom()}`);
        await expect(out()).toMatchFileSnapshot(snap("tree-callbacks"));
    });

    it("creates a new file in a folder from the tree's menu, and says nothing when it fails", async () => {
        const v = await mountView();
        const create = vi.spyOn(v.model, "createFile").mockResolvedValue(undefined as never);
        const dir = vi.spyOn(v.model, "createDir").mockRejectedValue(new Error("exists"));
        fireEvent.contextMenu(v.container.querySelector('[role="tree"]')!);
        fireEvent.pointerDown([...document.querySelectorAll(".ctx-menu-item")].find((n) => n.textContent?.includes("New File"))!);
        await settle();
        const input = () => v.container.querySelector<HTMLInputElement>(".file-tree-inline-input")!;
        fireEvent.input(input(), { target: { value: "new.ts" } });
        fireEvent.keyDown(input(), { key: "Enter" });
        await settle();
        fireEvent.contextMenu(v.container.querySelector('[role="tree"]')!);
        fireEvent.pointerDown([...document.querySelectorAll(".ctx-menu-item")].find((n) => n.textContent?.includes("New Folder"))!);
        await settle();
        fireEvent.input(input(), { target: { value: "dup" } });
        fireEvent.keyDown(input(), { key: "Enter" });
        await settle();
        h.log.push(`createFile=${JSON.stringify(create.mock.calls)} createDir=${JSON.stringify(dir.mock.calls)}`);
        fireEvent.contextMenu(v.container.querySelector('[role="tree"]')!);
        fireEvent.pointerDown([...document.querySelectorAll(".ctx-menu-item")].find((n) => n.textContent?.includes("New File"))!);
        await settle();
        fireEvent.keyDown(input(), { key: "Escape" });
        await settle();
        h.log.push(`input after cancel: ${input() != null}`);
        await expect(out()).toMatchFileSnapshot(snap("tree-new-entry"));
    });

    it("draws the tab bar with its tooltips, drag state and label while a tab is being saved as", async () => {
        const v = await mountView();
        await open(v.model, "/home/u/a.ts");
        await open(v.model, "/home/u/b.ts");
        await v.model.openScratch();
        await settle();
        const bar = () => v.container.querySelector(".editor-main-column > :first-child");
        const strip = (el: Element | null) => (el ? el.outerHTML : "(absent)").replace(/></g, ">\n<");
        h.log.push(strip(bar()));
        run(v.model, "editor:saveAs");
        await settle();
        h.log.push("-- while its Save As box is open");
        h.log.push(strip(bar()));
        await expect(out()).toMatchFileSnapshot(snap("tab-bar"));
    });

    it("opens the remote host picker over the pane and hands the choice to the model", async () => {
        const v = await mountView();
        const open = vi.spyOn(v.model, "openFromRemote").mockResolvedValue(undefined as never);
        h.log.push(`picker before: ${v.container.querySelector(".stub-remote") != null}`);
        v.model.setOpenFromRemote(true);
        await settle();
        h.log.push(`picker: ${v.container.querySelector(".stub-remote")?.outerHTML}`);
        fireEvent.click(v.container.querySelector(".stub-remote-open")!);
        h.log.push(`openFromRemote=${JSON.stringify(open.mock.calls)}`);
        fireEvent.click(v.container.querySelector(".stub-remote-close")!);
        h.log.push(`picker after close: ${v.container.querySelector(".stub-remote") != null} flag=${v.model.openFromRemoteAtom()}`);
        await expect(out()).toMatchFileSnapshot(snap("remote-picker"));
    });

    it("zooms the pane on Ctrl+scroll", async () => {
        const v = await mountView();
        await open(v.model, "/home/u/a.ts");
        const root = v.container.querySelector<HTMLElement>(".editor-view")!;
        h.log.push(`zoom before: ${v.model.zoomAtom()}`);
        for (const mod of [{ ctrlKey: true }, { metaKey: true }]) {
            const ev = new WheelEvent("wheel", { deltaY: -100, bubbles: true, cancelable: true, clientX: 10, clientY: 10, ...mod });
            root.dispatchEvent(ev);
            h.log.push(`wheel ${JSON.stringify(mod)} prevented=${ev.defaultPrevented}`);
        }
        await settle();
        h.log.push(`zoom after: ${v.model.zoomAtom()} style=${JSON.stringify(root.style.zoom)}`);
        await expect(out()).toMatchFileSnapshot(snap("zoom-wheel"));
    });

    it("Save As on a scratch tab also answers CodeMirror's own save-as key", async () => {
        const v = await mountView();
        await v.model.openScratch();
        await settle();
        h.log.push(`codemirror built: ${cm(v.model) != null}`);
        for (const mod of [{ ctrlKey: true, shiftKey: true }, { metaKey: true, shiftKey: true }]) {
            cm(v.model)?.contentDOM.dispatchEvent(new KeyboardEvent("keydown", { key: "S", code: "KeyS", bubbles: true, cancelable: true, ...mod }));
            await settle();
            h.log.push(`after ${JSON.stringify(mod)}: save-as input shown: ${v.container.querySelector(".editor-tab--saveas input") != null}`);
        }
        h.log.push(`save-as input shown: ${v.container.querySelector(".editor-tab--saveas input") != null}`);
        await expect(out()).toMatchFileSnapshot(snap("keys-save-as"));
    });
});

describe("clean-up", () => {
    it("unmounting leaves no runner, no confirm hook and no view behind", async () => {
        const v = await mountView();
        await open(v.model, "/home/u/a.ts");
        h.log.push(`confirm hook set: ${v.model.confirmDirtyClose != null}`);
        v.unmount();
        await settle();
        h.log.push(`confirm hook after: ${v.model.confirmDirtyClose != null} cmViewRef=${v.model.cmViewRef.current}`);
        await expect(out()).toMatchFileSnapshot(snap("cleanup"));
    });
});
