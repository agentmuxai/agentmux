// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What FilesView does, pinned as ordered logs: the order its effects and listeners are set up in,
 * the file-drop target, the drag handlers, opening, the `files:*` commands, the list's keys, the
 * grid, the menus and the windowed list. The real model runs against stand-in RPC; the modules the
 * view calls out to (drag, open, keybindings) log what they are asked. files-view.test.tsx holds the
 * rest of the behaviour and files-view.chrome.test.tsx the markup. A refactor of FilesView must leave
 * the committed snapshots as they are; regenerate with `vitest -u` only for a change meant to differ.
 */

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import type { FsEntry } from "@/types/rpc/FsEntry";

const h = vi.hoisted(() => {
    const state = {
        dirs: new Map<string, unknown[]>(),
        byType: new Map<string, ((event: unknown) => void)[]>(),
        trace: [] as string[],
        drop: undefined as any,
        runner: undefined as ((c: string) => boolean) | undefined,
        command: undefined as string | undefined,
        globalKey: false,
        drag: { paths: undefined as string[] | undefined, source: undefined as string | undefined },
        menuHandlers: {} as Record<string, any>,
        roCallbacks: [] as (() => void)[],
        target: "program" as string,
        fail: false,
        life: false,
        previewProps: undefined as any,
    };
    const rpc = {
        FsPlacesCommand: vi.fn(async () => ({
            home: "/home/a",
            sep: "/",
            places: [{ id: "home", label: "Home", path: "/home/a", kind: "home" }],
        })),
        FsListCommand: vi.fn(async (_c: unknown, req: { path: string }) => ({ path: req.path, entries: state.dirs.get(req.path) ?? [] })),
        FsWatchCommand: vi.fn(async () => ({ watch_id: "w1" })),
        FsUnwatchCommand: vi.fn(async () => ({})),
        FsTrashCommand: vi.fn(async (_c: unknown, req: { paths: string[] }) => ({ results: req.paths.map((path) => ({ path, ok: true })) })),
        FsGitStatusCommand: vi.fn(async () => ({ in_repo: false, entries: [], changes: 0 })),
        FsOpStartCommand: vi.fn(async () => ({ op_id: "op1" })),
        FsOpResolveCommand: vi.fn(async () => ({})),
        FsOpCancelCommand: vi.fn(async () => ({})),
        ListNamedAgentsCommand: vi.fn(async () => []),
        RemotesListCommand: vi.fn(async () => []),
        GetAgentContentCommand: vi.fn(async () => null),
    };
    return { state, rpc };
});

const T = h.state.trace;
const log = (line: string) => T.push(line);
/** The setup calls, logged only by the test that pins their order. */
const life = (line: string) => h.state.life && T.push(line);
const fmt = (v: unknown): string =>
    typeof v === "function"
        ? "fn"
        : v === undefined
          ? "undefined"
          : JSON.stringify(v, (_k, x) => (x instanceof Set ? [...x] : x && typeof x === "object" && "is_dir" in x ? `<${(x as FsEntry).name}>` : x));

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: h.rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: { rpcCall: vi.fn(async () => ({})) } }));
vi.mock("@/app/store/mps", () => {
    const mock = {
        muxEventSubscribe: (sub: { eventType: string; handler: (e: unknown) => void }) => {
            h.state.byType.set(sub.eventType, [...(h.state.byType.get(sub.eventType) ?? []), sub.handler]);
            return () => {};
        },
    };
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        mock.muxEventSubscribe({ eventType, scope, handler: (e: any) => { if (e?.data != null) handler(e.data, e); } } as any);
    return { ...mock, onMuxEvent };
});
vi.mock("./files-icons", async (orig) => ({ ...(await orig<typeof import("./files-icons")>()), systemIcon: async () => null }));
vi.mock("./files-thumbs", () => ({ thumbnail: async () => null, cachedThumbnail: () => null, hasThumbnail: () => false }));
vi.mock("./files-preview", () => ({
    FilesPreview: (p: any) => {
        h.state.previewProps = p;
        return <div class="stub-preview" data-entry={p.entry?.name ?? ""} />;
    },
}));
vi.mock("@/app/store/block-atom-cache", async (orig) => ({
    ...(await orig<typeof import("@/app/store/block-atom-cache")>()),
    getSettingsKeyAtom: () => () => undefined,
}));
vi.mock("@/app/store/pane-content-holds", () => ({
    trackPaneContent: (id: string) => (life(`trackPaneContent(${id})`), () => life("trackPaneContent disposed")),
    holdPaneContent: (id: string) => (life(`holdPaneContent(${id})`), () => life("pane content released")),
}));
vi.mock("@/app/drag/file-drop", async (orig) => ({
    ...(await orig<typeof import("@/app/drag/file-drop")>()),
    registerFileDropTarget: (id: string, target: unknown) => {
        life(`registerFileDropTarget(${id})`);
        h.state.drop = target;
        return () => life("file drop target disposed");
    },
    beginPathDrag: (_dt: unknown, paths: string[], source: string) => log(`beginPathDrag(${fmt(paths)}, ${source})`),
    endPathDrag: () => log("endPathDrag()"),
    pathDragPaths: () => h.state.drag.paths,
    pathDragSource: () => h.state.drag.source,
}));
vi.mock("@/app/keybindings", async (orig) => ({
    ...(await orig<typeof import("@/app/keybindings")>()),
    paneCommandFor: () => h.state.command,
    isGlobalKey: () => h.state.globalKey,
    registerPaneCommandRunner: (id: string, fn: (c: string) => boolean) => {
        life(`registerPaneCommandRunner(${id})`);
        h.state.runner = fn;
        return () => life("pane command runner disposed");
    },
}));
vi.mock("./files-agent-mention", async (orig) => ({
    ...(await orig<typeof import("./files-agent-mention")>()),
    trackAgentFocus: () => life("trackAgentFocus()"),
    mentionInAgent: (_m: unknown, list: unknown) => log(`mentionInAgent(${fmt(list)})`),
}));
vi.mock("./files-open", async (orig) => ({
    ...(await orig<typeof import("./files-open")>()),
    openTargetOf: () => h.state.target,
    openInPane: async (...a: unknown[]) => log(`openInPane(${fmt(a)})`),
    openInEditor: async (_m: unknown, path: string) => (log(`openInEditor(${path})`), h.state.fail && Promise.reject(new Error("boom"))),
    openWithOs: async (path: string, id: string) => (log(`openWithOs(${path}, ${id})`), h.state.fail && Promise.reject(new Error("boom"))),
    openEntryWithOs: (_m: unknown, name: string) => log(`openEntryWithOs(${name})`),
    openFolderInNewTab: async (...a: unknown[]) => (log(`openFolderInNewTab(${fmt(a)})`), h.state.fail && Promise.reject(new Error("boom"))),
    closeOwnTab: async (id: string) => {
        log(`closeOwnTab(${id})`);
        if (h.state.fail) throw new Error("boom");
        return h.state.target !== "only-tab";
    },
}));
vi.mock("./files-menus", async (orig) => {
    const real = await orig<typeof import("./files-menus")>();
    return {
        ...real,
        buildRowMenu: (m: any, e: any, handlers: any) => (log(`buildRowMenu(${e.name}, ${Object.keys(handlers).join(",")})`), (h.state.menuHandlers = handlers), real.buildRowMenu(m, e, handlers)),
        buildFolderMenu: (m: any, handlers: any) => (log(`buildFolderMenu(${Object.keys(handlers).join(",")})`), (h.state.menuHandlers = handlers), real.buildFolderMenu(m, handlers)),
    };
});

import { setPlatform } from "@/util/platformutil";
import { setClipboard } from "./files-ops";
import { FilesModel } from "./files-model";
import { FilesView } from "./files-view";

const entry = (name: string, over: Partial<FsEntry> = {}): FsEntry => ({ name, is_dir: false, is_symlink: false, hidden: false, readonly: false, size: 10, mtime: 0, ...over });
const HOME = "/home/a";
const snap = (name: string) => `./__snapshots__/files-view.behavior.${name}.txt`;
const out = () => T.join("\n") + "\n";

class FakeResizeObserver {
    constructor(public cb: () => void) {
        h.state.roCallbacks.push(cb);
    }
    observe() {
        life("ResizeObserver.observe(list)");
    }
    disconnect() {
        life("ResizeObserver.disconnect()");
    }
}

function mount(meta: Record<string, unknown> = {}, prep?: (m: FilesModel) => void) {
    const [m, setM] = createSignal<Record<string, unknown>>(meta);
    const [vis, setVis] = createSignal<"active" | "dormant">("active");
    const ctx: PaneTabHostContext = {
        blockId: "b1",
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
        visibility: () => vis(),
    };
    const model = new FilesModel(ctx);
    prep?.(model);
    const r = render(() => <FilesView model={model} ctx={ctx} />);
    const q = (sel: string) => r.container.querySelector(sel) as HTMLElement;
    return { ...r, model, ctx, setM, setVis, view: () => q(".files-view"), list: () => q(".files-list"), q };
}
type Mounted = ReturnType<typeof mount>;

const rowNames = (v: Mounted) => [...v.container.querySelectorAll(".files-row .files-name, .files-tile .files-tile-name")].map((e) => e.textContent);
const settled = async (v: Mounted) => await waitFor(() => expect(rowNames(v).length).toBeGreaterThan(0));
const row = (v: Mounted, name: string) =>
    [...v.container.querySelectorAll(".files-row:not(.files-header), .files-tile")].find((e) => e.textContent?.includes(name)) as HTMLElement;
/** Replaces the model's methods with logging stand-ins (they answer `ret`). */
const stub = (model: FilesModel, names: string[], ret?: unknown) =>
    names.forEach((n) =>
        vi.spyOn(model as any, n).mockImplementation((...a: unknown[]) => {
            log(`model.${n}(${a.map(fmt).join(", ")})`);
            return ret;
        })
    );
/** Logs a model method's calls and lets it run. */
const watch = (model: FilesModel, names: string[]) =>
    names.forEach((n) => {
        const orig = (model as any)[n].bind(model);
        vi.spyOn(model as any, n).mockImplementation((...a: unknown[]) => {
            log(`model.${n}(${a.map(fmt).join(", ")})`);
            return orig(...a);
        });
    });
const sel = (v: Mounted) => `selection=${fmt(v.model.selection())}`;
const key = (el: Element, k: string, init: KeyboardEventInit = {}) => fireEvent.keyDown(el, { key: k, ...init });

beforeEach(() => {
    setPlatform("linux");
    h.state.dirs.clear();
    h.state.byType = new Map();
    h.state.roCallbacks = [];
    h.state.command = undefined;
    h.state.globalKey = false;
    h.state.drag = { paths: undefined, source: undefined };
    h.state.target = "program";
    h.state.fail = false;
    h.state.life = false;
    T.length = 0;
    setClipboard(null);
    h.state.dirs.set(HOME, [entry("b.txt"), entry("src", { is_dir: true, size: undefined }), entry("a.md"), entry("run.exe"), entry("pic.png")]);
    h.state.dirs.set(`${HOME}/src`, [entry("inner.ts")]);
    h.state.dirs.set("~", h.state.dirs.get(HOME)!);
    for (const fn of Object.values(h.rpc)) fn.mockClear();
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
});
afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setPlatform("darwin");
});

describe("FilesView setup", () => {
    it("sets up its holds, listeners and effects in this order, and answers the pane's later changes", async () => {
        h.state.life = true;
        const add = vi.spyOn(window, "addEventListener");
        const timer = vi.spyOn(globalThis, "setInterval");
        const v = mount({ "files:select": ["a.md"] }, (m) => watch(m, ["start", "onShown", "applyRequestedSelection", "navigate"]));
        for (const [type, , opts] of add.mock.calls) if (type === "dragend") log(`window.addEventListener(dragend, capture=${opts === true})`);
        for (const [, ms] of timer.mock.calls) if (ms === 60_000) log("setInterval(60000)");
        await settled(v);
        log(`> listed: ${fmt(rowNames(v))}`);
        log("> the pane is hidden, then shown");
        v.setVis("dormant");
        v.setVis("active");
        log("> the select request changes");
        v.setM({ "files:select": ["b.txt"] });
        log("> a folder is opened with the filter open");
        const list = v.list();
        Object.defineProperty(list, "scrollHeight", { value: 900, configurable: true });
        list.scrollTop = 50;
        await v.model.navigate(`${HOME}/src`);
        log(`> opened: path=${v.model.path()} listScrollTop=${list.scrollTop} rows=${fmt(rowNames(v))}`);
        log("> the list's size changes");
        Object.defineProperty(list, "clientHeight", { value: 240, configurable: true });
        Object.defineProperty(list, "clientWidth", { value: 450, configurable: true });
        h.state.roCallbacks.forEach((cb) => cb());
        log("> unmounted");
        v.unmount();
        const dragendRemoved = add.mock.calls.filter(([t]) => t === "dragend").length;
        log(`dragend listeners added: ${dragendRemoved}`);
        await expect(out()).toMatchFileSnapshot(snap("setup"));
    });
});

const geometry = (el: HTMLElement, g: { clientHeight?: number; clientWidth?: number; scrollHeight?: number } = {}) => {
    let top = 0;
    const dims = { clientHeight: 120, clientWidth: 600, scrollHeight: 5000, ...g };
    for (const [k, v] of Object.entries(dims)) Object.defineProperty(el, k, { configurable: true, get: () => v });
    Object.defineProperty(el, "scrollTop", {
        configurable: true,
        get: () => top,
        set: (v: number) => {
            top = v;
            log(`list.scrollTop = ${v}`);
        },
    });
};
/** jsdom's DragEvent drops `relatedTarget`, so set it on a plain event. */
const leave = (el: Element, related: Element) => {
    const e = new Event("dragleave", { bubbles: true });
    Object.defineProperty(e, "relatedTarget", { value: related });
    el.dispatchEvent(e);
};
const aux = (el: Element, button: number) => fireEvent(el, new MouseEvent("auxclick", { bubbles: true, cancelable: true, button }));
const flush = () => new Promise<void>((r) => setTimeout(r, 0));
const dialog = () => document.body.querySelector(".confirm-dialog");
const dialogButton = (label: string) => [...document.body.querySelectorAll(".confirm-dialog button")].find((b) => b.textContent === label) as HTMLElement;
const menuLabels = () => [...document.body.querySelectorAll(".ctx-menu .ctx-menu-label")].map((e) => e.textContent);
const select = (v: Mounted, ...names: string[]) => v.model.setSelection({ names: new Set(names), focus: names[names.length - 1], anchor: names[0] });

describe("FilesView drops and drags", () => {
    it("accepts and drops files as the drop target, and tracks the folder row under a drag", async () => {
        const v = mount();
        const target = h.state.drop;
        const early = fmt(target.accept({ count: 1, names: ["x.txt"] }));
        await settled(v);
        T.length = 0;
        log(`before listing: ${early}`);
        stub(v.model, ["transfer"], Promise.resolve());
        watch(v.model, ["setStatus"]);
        const accept = (label: string, drag: { count: number; names?: string[] }) => log(`accept ${label} = ${fmt(target.accept(drag))}`);
        accept("one name", { count: 1, names: ["x.txt"] });
        accept("one, no name", { count: 1 });
        accept("three", { count: 3 });
        h.state.drag = { paths: ["/other/a"], source: "b2" };
        accept("in-app paths", { count: 1, names: ["a"] });
        h.state.drag = { paths: ["/home/a/b.txt"], source: "b1" };
        accept("from this pane", { count: 1, names: ["b.txt"] });
        target.onNoPaths();
        log("> dropping");
        h.state.drag = { paths: ["/other/a"], source: "b2" };
        await target.drop({ paths: [] });
        await target.drop({ paths: ["/other/a", "/other/b"] });
        h.state.drag = { paths: ["/home/a/b.txt"], source: "b1" };
        await target.drop({ paths: ["/home/a/b.txt"] });
        log("> dragging over a folder row, then dropping");
        h.state.drag = { paths: ["/other/a"], source: "b2" };
        fireEvent.dragOver(row(v, "src"));
        log(`drop target row: ${fmt([...v.container.querySelectorAll(".files-row-droptarget .files-name")].map((e) => e.textContent))}`);
        await target.drop({ paths: ["/other/a"] });
        log(`after the drop: ${v.container.querySelectorAll(".files-row-droptarget").length}`);
        log("> over a file row, a folder row being dragged, blank space, and leaving");
        fireEvent.dragOver(row(v, "b.txt"));
        log(`file row: ${v.container.querySelectorAll(".files-row-droptarget").length}`);
        h.state.drag = { paths: ["/home/a/src"], source: "b1" };
        fireEvent.dragOver(row(v, "src"));
        log(`folder being dragged: ${v.container.querySelectorAll(".files-row-droptarget").length}`);
        h.state.drag = { paths: ["/other/a"], source: "b2" };
        fireEvent.dragOver(row(v, "src"));
        fireEvent.dragOver(v.list());
        log(`blank space: ${v.container.querySelectorAll(".files-row-droptarget").length}`);
        fireEvent.dragOver(row(v, "src"));
        leave(v.list(), row(v, "src"));
        log(`leave into a row: ${v.container.querySelectorAll(".files-row-droptarget").length}`);
        leave(v.list(), document.body);
        log(`leave out: ${v.container.querySelectorAll(".files-row-droptarget").length}`);
        fireEvent.dragOver(row(v, "src"));
        window.dispatchEvent(new Event("dragend"));
        log(`window dragend: ${v.container.querySelectorAll(".files-row-droptarget").length}`);
        await expect(out()).toMatchFileSnapshot(snap("drop"));
    });

    it("starts a drag with the selected paths, and refuses one on a host or in a confined folder", async () => {
        const v = mount();
        await settled(v);
        T.length = 0;
        const dt = { setData() {}, effectAllowed: "" };
        log(`no data transfer: ${fireEvent.dragStart(row(v, "b.txt"))} ${sel(v)}`);
        log(`one row: ${fireEvent.dragStart(row(v, "b.txt"), { dataTransfer: dt })} ${sel(v)}`);
        select(v, "a.md", "b.txt");
        log(`a selected row of two: ${fireEvent.dragStart(row(v, "b.txt"), { dataTransfer: dt })} ${sel(v)}`);
        fireEvent.dragEnd(row(v, "b.txt"));
        cleanup();
        const host = mount({ connection: "user@box" });
        await settled(host);
        log(`on a host: notPrevented=${fireEvent.dragStart(row(host, "b.txt"), { dataTransfer: dt })}`);
        cleanup();
        const confined = mount({ "files:root": HOME, "files:path": HOME });
        await settled(confined);
        log(`confined: notPrevented=${fireEvent.dragStart(row(confined, "b.txt"), { dataTransfer: dt })}`);
        await expect(out()).toMatchFileSnapshot(snap("drag"));
    });
});

describe("FilesView opening", () => {
    it("opens a folder, an editor file, an OS file and a program, here and on a host, and says why when it can't", async () => {
        const v = mount();
        await settled(v);
        T.length = 0;
        stub(v.model, ["navigate"], Promise.resolve());
        watch(v.model, ["setStatus"]);
        const open = (name: string) => fireEvent.dblClick(row(v, name));
        open("src");
        h.state.target = "editor";
        open("a.md");
        h.state.target = "os";
        open("pic.png");
        h.state.target = "program";
        open("run.exe");
        log(`dialog: ${dialog()?.textContent}`);
        fireEvent.click(dialogButton("Cancel"));
        log(`cancelled: dialog=${!!dialog()} focusOnList=${document.activeElement === v.list()}`);
        open("run.exe");
        fireEvent.click(dialogButton("Run"));
        log(`confirmed: dialog=${!!dialog()} focusOnList=${document.activeElement === v.list()}`);
        h.state.fail = true;
        log("> failures");
        h.state.target = "editor";
        open("a.md");
        h.state.target = "os";
        open("pic.png");
        await flush();
        h.state.target = "program";
        open("run.exe");
        fireEvent.click(dialogButton("Run"));
        await flush();
        cleanup();
        h.state.fail = false;
        T.push("> on a host");
        const host = mount({ connection: "user@box" });
        await settled(host);
        h.state.target = "editor";
        fireEvent.dblClick(row(host, "a.md"));
        h.state.target = "os";
        watch(host.model, ["notOnHost"]);
        fireEvent.dblClick(row(host, "pic.png"));
        cleanup();
        T.push("> in a confined folder");
        const confined = mount({ "files:root": HOME, "files:path": HOME });
        await settled(confined);
        watch(confined.model, ["refuseInRoot"]);
        h.state.target = "os";
        fireEvent.dblClick(row(confined, "pic.png"));
        h.state.target = "editor";
        fireEvent.dblClick(row(confined, "a.md"));
        await expect(out()).toMatchFileSnapshot(snap("open"));
    });
});

describe("FilesView commands", () => {
    it("runs every files command the runner is given, and says false for one it doesn't know", async () => {
        const writes: string[] = [];
        Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async (t: string) => (writes.push(t), h.state.fail && Promise.reject(new Error("no"))) } });
        const v = mount();
        await settled(v);
        T.length = 0;
        stub(v.model, ["goBack", "goForward", "goUp", "setRenaming", "refresh", "trash", "undo", "copyToClipboard", "paste", "createNew", "deletePermanently", "navigate"], Promise.resolve());
        watch(v.model, ["setSelection", "setStatus"]);
        select(v, "b.txt");
        const run = (c: string) => log(`${c} -> ${h.state.runner!(c)}`);
        for (const c of ["files:back", "files:forward", "files:up", "files:rename", "files:refresh", "files:trash", "files:selectAll", "files:undo", "files:copy", "files:cut", "files:paste", "files:newFolder", "files:newTabHere"]) run(c);
        select(v, "src", "b.txt");
        h.state.target = "editor";
        run("files:openInNewTab");
        h.state.target = "program";
        run("files:rename");
        run("files:deletePermanently");
        log(`dialog: ${dialog()?.textContent}`);
        fireEvent.click(dialogButton("Delete permanently"));
        select(v);
        run("files:deletePermanently");
        log(`dialog with nothing selected: ${!!dialog()}`);
        run("files:closeTab");
        await flush();
        h.state.target = "only-tab";
        run("files:closeTab");
        await flush();
        h.state.fail = true;
        run("files:closeTab");
        await flush();
        select(v, "b.txt");
        run("files:copy");
        await flush();
        h.state.fail = false;
        run("files:copy");
        await flush();
        log(`clipboard writes: ${fmt(writes)}`);
        run("files:mention");
        run("files:editPath");
        await flush();
        log(`path box open: ${!!v.q(".files-path-input")} focused=${document.activeElement === v.q(".files-path-input")}`);
        run("files:filter");
        await flush();
        log(`filter box open: ${!!v.q(".files-filter-input")} focused=${document.activeElement === v.q(".files-filter-input")}`);
        run("files:nope");
        await expect(out()).toMatchFileSnapshot(snap("commands"));
    });
});

describe("FilesView keys", () => {
    it("moves, opens, filters and types ahead in the list, and leaves the keys it doesn't use", async () => {
        const v = mount();
        await settled(v);
        T.length = 0;
        geometry(v.list());
        stub(v.model, ["goBack", "togglePreview", "navigate", "refresh"], Promise.resolve());
        const press = (k: string, init: KeyboardEventInit = {}, el: Element = v.list()) => {
            const notPrevented = key(el, k, init);
            const mods = ["ctrlKey", "shiftKey", "altKey", "metaKey"].filter((m) => (init as any)[m]).map((m) => m[0]).join("");
            log(`${k}${mods ? "+" + mods : ""}: handled=${!notPrevented} ${sel(v)}`);
        };
        for (const k of ["ArrowDown", "ArrowDown", "ArrowDown", "ArrowUp", "End", "Home", "PageDown", "PageUp"]) press(k);
        press("ArrowDown", { shiftKey: true });
        press("ArrowDown", { ctrlKey: true });
        press(" ", { ctrlKey: true });
        press(" ");
        log("> Enter on a folder, then on two files");
        select(v, "src");
        press("Enter");
        h.state.target = "editor";
        select(v, "a.md", "b.txt", "src");
        press("Enter");
        press("Backspace");
        log("> the filter");
        press("/");
        log(`filter box: ${!!v.q(".files-filter-input")}`);
        v.model.setFilter("a");
        press("Escape");
        log(`after Escape: filter=${JSON.stringify(v.model.filter())} box=${!!v.q(".files-filter-input")}`);
        press("Escape");
        log("> type-ahead");
        press("p");
        press("/");
        press(" ");
        log("> keys it doesn't use");
        press("F9");
        press("a", { ctrlKey: true });
        h.state.globalKey = true;
        press("ArrowDown", { ctrlKey: true });
        press("ArrowDown", { metaKey: true });
        h.state.globalKey = false;
        log("> a files command key");
        h.state.command = "files:refresh";
        press("F5");
        press("ArrowDown", { ctrlKey: true });
        h.state.command = undefined;
        log("> while renaming");
        v.model.setRenaming("b.txt");
        press("ArrowDown");
        v.model.setRenaming(null);
        log("> while editing the path");
        h.state.runner!("files:editPath");
        press("ArrowDown");
        await expect(out()).toMatchFileSnapshot(snap("keys"));
    });

    it("answers command keys from anywhere in the pane except a text field, the list or a rename", async () => {
        const v = mount();
        await settled(v);
        T.length = 0;
        stub(v.model, ["refresh"], Promise.resolve());
        const field = document.createElement("input");
        v.view().appendChild(field);
        const press = (label: string, el: Element) => log(`${label}: handled=${!key(el, "F5")}`);
        press("no command", v.q(".files-tool")!);
        h.state.command = "files:refresh";
        press("on the toolbar", v.q(".files-tool")!);
        press("in a text field", field);
        press("in the list", v.list());
        v.model.setRenaming("b.txt");
        press("while renaming", v.q(".files-tool")!);
        v.model.setRenaming(null);
        h.state.command = "files:unknown";
        press("an unknown command", v.q(".files-tool")!);
        await expect(out()).toMatchFileSnapshot(snap("pane-keys"));
    });

    it("moves by tiles in the grid, keeps the focused tile in view and pages by whole rows", async () => {
        const v = mount({ "files:view": "grid" });
        await settled(v);
        T.length = 0;
        geometry(v.list(), { clientHeight: 270 });
        h.state.roCallbacks.forEach((cb) => cb());
        const press = (k: string, init: KeyboardEventInit = {}) => log(`${k}${init.altKey ? "+a" : ""}: handled=${!key(v.list(), k, init)} ${sel(v)}`);
        for (const k of ["ArrowRight", "ArrowDown", "ArrowLeft", "ArrowUp", "PageDown", "End", "PageUp", "Home"]) press(k);
        press("ArrowRight", { altKey: true });
        await expect(out()).toMatchFileSnapshot(snap("grid-keys"));
    });
});

describe("FilesView pointer", () => {
    it("selects with clicks and modifiers, opens menus, and opens a folder in a new tab with the middle button", async () => {
        const v = mount();
        await settled(v);
        T.length = 0;
        const writes: string[] = [];
        Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async (t: string) => void writes.push(t) } });
        stub(v.model, ["deletePermanently"], Promise.resolve());
        watch(v.model, ["setStatus"]);
        const click = (name: string, init: MouseEventInit = {}) => {
            fireEvent.click(row(v, name), init);
            log(`click ${name} ${JSON.stringify(init)}: ${sel(v)} focusOnList=${document.activeElement === v.list()}`);
        };
        click("b.txt");
        click("pic.png", { ctrlKey: true });
        click("src", { shiftKey: true });
        fireEvent.click(v.list());
        log(`blank space: ${sel(v)}`);
        log("> the row menu");
        log(`contextmenu notPrevented=${fireEvent.contextMenu(row(v, "run.exe"))} ${sel(v)}`);
        log(`menu: ${fmt(menuLabels())}`);
        const handlers = h.state.menuHandlers;
        handlers.openInNewTab("/home/a/src");
        handlers.copyPaths([entry("b.txt"), entry("a.md")]);
        handlers.copyPaths([entry("b.txt")], true);
        await flush();
        handlers.askDeletePermanently([entry("b.txt")]);
        log(`dialog: ${dialog()?.textContent}`);
        fireEvent.click(dialogButton("Cancel"));
        handlers.askDeletePermanently([]);
        log(`empty list asks: ${!!dialog()}`);
        h.state.target = "editor";
        handlers.openEntry(entry("a.md"));
        log(`clipboard: ${fmt(writes)}`);
        log("> the folder menu");
        cleanup();
        const w = mount();
        await settled(w);
        log(`blank contextmenu notPrevented=${fireEvent.contextMenu(w.list())}`);
        log(`menu: ${fmt(menuLabels())}`);
        h.state.menuHandlers.openInNewTab("/home/a");
        log(`pane contextmenu notPrevented=${fireEvent.contextMenu(w.view())}`);
        log("> the middle button");
        log(`mousedown on a folder notPrevented=${fireEvent.mouseDown(row(w, "src"), { button: 1 })}`);
        log(`mousedown on a file notPrevented=${fireEvent.mouseDown(row(w, "b.txt"), { button: 1 })}`);
        log(`mousedown left notPrevented=${fireEvent.mouseDown(row(w, "src"), { button: 0 })}`);
        log(`auxclick on a folder notPrevented=${aux(row(w, "src"), 1)}`);
        log(`auxclick on a file notPrevented=${aux(row(w, "b.txt"), 1)}`);
        log(`auxclick right notPrevented=${aux(row(w, "src"), 2)}`);
        log("> sorting");
        const size = [...w.container.querySelectorAll(".files-header button")].find((b) => b.textContent === "Size") as HTMLElement;
        fireEvent.click(size);
        log(`sort=${w.model.sortKey()} ${w.model.sortDir()} aria=${size.getAttribute("aria-sort")} rows=${fmt(rowNames(w))}`);
        await expect(out()).toMatchFileSnapshot(snap("pointer"));
    });

    it("shows tiles in the grid with the same handlers, and goes back to the top when the view changes", async () => {
        const v = mount({ "files:view": "grid" });
        await settled(v);
        T.length = 0;
        geometry(v.list());
        stub(v.model, ["navigate"], Promise.resolve());
        log(`tiles: ${fmt(rowNames(v))} grid=${!!v.q(".files-grid")} rows=${!!v.q(".files-rows")}`);
        const dt = { setData() {}, effectAllowed: "" };
        fireEvent.click(row(v, "a.md"));
        log(sel(v));
        h.state.target = "editor";
        fireEvent.dblClick(row(v, "a.md"));
        log(`contextmenu notPrevented=${fireEvent.contextMenu(row(v, "pic.png"))} menu=${fmt(menuLabels())}`);
        log(`dragstart notPrevented=${fireEvent.dragStart(row(v, "pic.png"), { dataTransfer: dt })}`);
        h.state.drag = { paths: ["/other/a"], source: "b2" };
        fireEvent.dragOver(row(v, "src"));
        log(`drop target: ${v.container.querySelectorAll(".files-row-droptarget").length}`);
        fireEvent.dragEnd(row(v, "src"));
        v.list().scrollTop = 99;
        log("> to the list");
        fireEvent.click(v.container.querySelector('[aria-label="Show as a list"]')!);
        log(`mode=${v.model.viewMode()} rows=${!!v.q(".files-rows")} grid=${!!v.q(".files-grid")}`);
        log("> to the grid");
        fireEvent.click(v.container.querySelector('[aria-label="Show as thumbnails"]')!);
        log(`mode=${v.model.viewMode()} grid=${!!v.q(".files-grid")}`);
        await expect(out()).toMatchFileSnapshot(snap("grid"));
    });
});

describe("FilesView path and filter", () => {
    it("edits the path in place, commits it only when it changed, and closes on Escape or blur", async () => {
        const v = mount();
        await settled(v);
        T.length = 0;
        stub(v.model, ["navigate"], Promise.resolve());
        const edit = async () => {
            fireEvent.dblClick(v.q(".files-breadcrumb"));
            await flush();
            return v.q(".files-path-input") as HTMLInputElement;
        };
        let input = await edit();
        log(`editing: ${!!input} value=${input.value} focused=${document.activeElement === input}`);
        input.value = "  /home/a/src  ";
        key(input, "Enter");
        log(`committed: input=${!!v.q(".files-path-input")} crumbs=${!!v.q(".files-breadcrumb")} focusOnList=${document.activeElement === v.list()}`);
        input = await edit();
        key(input, "Enter");
        log(`same path: input=${!!v.q(".files-path-input")}`);
        input = await edit();
        input.value = "   ";
        key(input, "Enter");
        input = await edit();
        key(input, "Escape");
        log(`Escape: input=${!!v.q(".files-path-input")} focusOnList=${document.activeElement === v.list()}`);
        input = await edit();
        fireEvent.blur(input);
        log(`blur: input=${!!v.q(".files-path-input")}`);
        await expect(out()).toMatchFileSnapshot(snap("path"));
    });

    it("opens the filter box, hands the focus to the list and closes with the filter", async () => {
        const v = mount();
        await settled(v);
        T.length = 0;
        fireEvent.click(v.container.querySelector('[aria-label^="Filter ("]')!);
        await flush();
        const box = v.q(".files-filter-input") as HTMLInputElement;
        log(`open: focused=${document.activeElement === box}`);
        box.value = "a";
        fireEvent.input(box);
        log(`filter=${JSON.stringify(v.model.filter())} rows=${fmt(rowNames(v))} status=${v.q(".files-status")?.textContent}`);
        key(box, "ArrowDown");
        log(`ArrowDown: ${sel(v)} focusOnList=${document.activeElement === v.list()}`);
        key(box, "Escape");
        await flush();
        log(`Escape: filter=${JSON.stringify(v.model.filter())} box=${!!v.q(".files-filter-input")} rows=${fmt(rowNames(v))} focusOnList=${document.activeElement === v.list()}`);
        log("> navigating clears the filter box");
        fireEvent.click(v.container.querySelector('[aria-label^="Filter ("]')!);
        await v.model.navigate(`${HOME}/src`);
        log(`box=${!!v.q(".files-filter-input")}`);
        await expect(out()).toMatchFileSnapshot(snap("filter"));
    });
});

describe("FilesView filter button and column dividers", () => {
    const button = (v: Mounted) => v.container.querySelector(".files-filter-button") as HTMLElement;
    const box = (v: Mounted) => v.q(".files-filter-input") as HTMLInputElement | null;
    const type = (v: Mounted, text: string) => {
        const input = box(v)!;
        input.value = text;
        fireEvent.input(input);
    };
    const pointer = (el: Element, type: string, clientX = 0) => el.dispatchEvent(new MouseEvent(type, { bubbles: true, clientX }));

    it("the filter button toggles the box, and says so", async () => {
        const v = mount();
        await settled(v);
        expect(box(v)).toBeNull();
        expect(button(v).getAttribute("aria-pressed")).toBe("false");
        fireEvent.click(button(v));
        await flush();
        expect(box(v)).not.toBeNull();
        expect(document.activeElement).toBe(box(v));
        expect(button(v).getAttribute("aria-pressed")).toBe("true");
        fireEvent.click(button(v));
        await flush();
        expect(box(v)).toBeNull();
        expect(button(v).getAttribute("aria-pressed")).toBe("false");
    });

    it("hiding the box keeps the filter applied, and the button says so; showing it again shows the same text", async () => {
        const v = mount();
        await settled(v);
        fireEvent.click(button(v));
        await flush();
        type(v, "a");
        const filtered = rowNames(v);
        fireEvent.click(button(v));
        await flush();
        expect(box(v)).toBeNull();
        expect(v.model.filter()).toBe("a");
        expect(rowNames(v)).toEqual(filtered);
        expect(button(v).className).toContain("files-filter-button--applied");
        expect(button(v).getAttribute("aria-pressed")).toBe("true");
        expect(button(v).getAttribute("aria-label")).toBe('Filter: "a" (click to edit)');
        fireEvent.click(button(v));
        await flush();
        expect(box(v)!.value).toBe("a");
        expect(button(v).className).not.toContain("files-filter-button--applied");
        // Escape clears the text and hides the box.
        key(box(v)!, "Escape");
        await flush();
        expect(box(v)).toBeNull();
        expect(v.model.filter()).toBe("");
        expect(button(v).className).not.toContain("files-filter-button--applied");
    });

    it("a press outside the box and the button hides the box and keeps the filter", async () => {
        const v = mount();
        await settled(v);
        fireEvent.click(button(v));
        await flush();
        type(v, "a");
        pointer(box(v)!, "pointerdown");
        pointer(button(v), "pointerdown");
        expect(box(v)).not.toBeNull();
        pointer(v.list(), "pointerdown");
        await flush();
        expect(box(v)).toBeNull();
        expect(v.model.filter()).toBe("a");
        expect(button(v).className).toContain("files-filter-button--applied");
        // With the box hidden nothing listens for presses any more: a press elsewhere changes nothing.
        pointer(v.list(), "pointerdown");
        expect(v.model.filter()).toBe("a");
    });

    it("files:filter toggles the box like the button", async () => {
        const v = mount();
        await settled(v);
        h.state.runner!("files:filter");
        await flush();
        expect(box(v)).not.toBeNull();
        h.state.runner!("files:filter");
        await flush();
        expect(box(v)).toBeNull();
    });

    const modifiedDivider = (v: Mounted) => v.container.querySelector(".files-header .files-col-modified .files-col-resizer") as HTMLElement;
    const listVar = (v: Mounted, name: string) => v.list().style.getPropertyValue(name);

    it("dragging a column divider resizes the column within its limits, keeps it in meta on release, and double-click resets it", async () => {
        const v = mount();
        await settled(v);
        const handle = modifiedDivider(v);
        vi.spyOn(handle.parentElement!, "getBoundingClientRect").mockReturnValue({ width: 110 } as DOMRect);
        pointer(handle, "pointerdown", 500);
        pointer(handle, "pointermove", 540);
        expect(listVar(v, "--files-w-modified")).toBe("150px");
        // Live only: nothing written yet.
        expect(v.ctx.meta()?.["files:colwidths"]).toBeUndefined();
        pointer(handle, "pointermove", 0);
        expect(listVar(v, "--files-w-modified")).toBe("60px");
        pointer(handle, "pointermove", 560);
        pointer(handle, "pointerup", 560);
        expect(listVar(v, "--files-w-modified")).toBe("170px");
        expect(v.ctx.meta()?.["files:colwidths"]).toEqual({ modified: 170 });
        // A divider's press and click are not a sort.
        const before = v.model.sortKey();
        fireEvent.click(handle);
        expect(v.model.sortKey()).toBe(before);
        fireEvent.dblClick(handle);
        expect(listVar(v, "--files-w-modified")).toBe("");
        expect(v.ctx.meta()?.["files:colwidths"]).toBeUndefined();
    });

    it("keeps a released width on screen while the meta write is in flight, then lets the meta show it", async () => {
        const v = mount();
        await settled(v);
        const real = v.ctx.setMeta;
        let finish: () => void = () => {};
        v.ctx.setMeta = (patch) => new Promise<void>((resolve) => (finish = () => resolve(real(patch))));
        const handle = modifiedDivider(v);
        vi.spyOn(handle.parentElement!, "getBoundingClientRect").mockReturnValue({ width: 110 } as DOMRect);
        pointer(handle, "pointerdown", 500);
        pointer(handle, "pointermove", 560);
        pointer(handle, "pointerup", 560);
        // Released, but the meta has not changed yet: the column does not snap back.
        expect(v.ctx.meta()?.["files:colwidths"]).toBeUndefined();
        expect(listVar(v, "--files-w-modified")).toBe("170px");
        finish();
        await flush();
        expect(v.ctx.meta()?.["files:colwidths"]).toEqual({ modified: 170 });
        expect(listVar(v, "--files-w-modified")).toBe("170px");
        // The same for a reset: the column's own width at once, not the old one until the meta follows.
        fireEvent.dblClick(handle);
        expect(listVar(v, "--files-w-modified")).toBe("");
        expect(v.ctx.meta()?.["files:colwidths"]).toEqual({ modified: 170 });
        finish();
        await flush();
        expect(v.ctx.meta()?.["files:colwidths"]).toBeUndefined();
        expect(listVar(v, "--files-w-modified")).toBe("");
    });

    it("draws a pane's stored column widths, and ignores ones that are not numbers", async () => {
        const v = mount({ "files:colwidths": { size: 100, kind: "wide", name: 300 } });
        await settled(v);
        expect(listVar(v, "--files-w-size")).toBe("100px");
        expect(listVar(v, "--files-w-kind")).toBe("");
        expect(listVar(v, "--files-flex-name")).toBe("0 0 300px");
    });
});

describe("FilesView windowing, preview and touches", () => {
    it("draws only the rows in view, moves the window on scroll and resize, and reveals a requested row", async () => {
        h.state.dirs.set("/big", Array.from({ length: 300 }, (_, i) => entry(`f${String(i).padStart(3, "0")}`)));
        const v = mount({ "files:path": "/big" });
        await settled(v);
        T.length = 0;
        const list = v.list();
        geometry(list, { clientHeight: 240 });
        const view = (title: string) => {
            const names = rowNames(v) as string[];
            log(`${title}: ${names.length} rows ${names[0]}..${names[names.length - 1]} height=${(v.q(".files-rows") ?? v.q(".files-grid"))?.style.height} rowcount=${list.getAttribute("aria-rowcount")}`);
        };
        view("start");
        h.state.roCallbacks.forEach((cb) => cb());
        view("after the first resize");
        fireEvent.scroll(list, { target: { scrollTop: 2400 } });
        view("scrolled to 2400");
        fireEvent.scroll(list, { target: { scrollTop: 2410 } });
        view("scrolled 10 more");
        log(`active descendant: ${list.getAttribute("aria-activedescendant")}`);
        log("> a reveal request");
        v.setM({ "files:path": "/big", "files:select": ["f250"] });
        log(`${sel(v)} range ${(rowNames(v) as string[])[0]}..${(rowNames(v) as string[]).at(-1)} descendant=${list.getAttribute("aria-activedescendant")}`);
        log("> the grid");
        cleanup();
        const g = mount({ "files:path": "/big", "files:view": "grid" });
        await settled(g);
        const gl = g.list();
        geometry(gl, { clientHeight: 270, clientWidth: 450 });
        h.state.roCallbacks.forEach((cb) => cb());
        const gnames = rowNames(g) as string[];
        log(`grid start: ${gnames.length} tiles ${gnames[0]}..${gnames[gnames.length - 1]} height=${g.q(".files-grid")?.style.height}`);
        fireEvent.scroll(gl, { target: { scrollTop: 1320 } });
        const g2 = rowNames(g) as string[];
        log(`grid scrolled: ${g2.length} tiles ${g2[0]}..${g2[g2.length - 1]}`);
        await expect(out()).toMatchFileSnapshot(snap("window"));
    });

    it("shows the preview for one selected row, the summary for the selection, and agent touches until they expire", async () => {
        const { noteToolCall, noteToolResult, resetTouchedForTests } = await import("@/app/store/touched-files");
        resetTouchedForTests();
        let tick: (() => void) | undefined;
        const realSet = globalThis.setInterval;
        vi.spyOn(globalThis, "setInterval").mockImplementation(((fn: () => void, ms?: number, ...rest: unknown[]) => {
            if (ms === 60_000) {
                tick = fn;
                return 0 as any;
            }
            return (realSet as any)(fn, ms, ...rest);
        }) as any);
        const v = mount({ "files:preview": true });
        await settled(v);
        T.length = 0;
        const summary = () => v.q(".files-status")?.textContent;
        log(`nothing selected: preview=${JSON.stringify(v.q(".stub-preview")?.getAttribute("data-entry"))} ${summary()}`);
        select(v, "b.txt");
        log(`one: preview=${JSON.stringify(v.q(".stub-preview")?.getAttribute("data-entry"))} ${summary()}`);
        select(v, "b.txt", "src");
        log(`two: preview=${JSON.stringify(v.q(".stub-preview")?.getAttribute("data-entry"))} ${summary()}`);
        v.model.setFilter("a");
        log(`filtered: ${summary()}`);
        v.model.setFilter("");
        log("> touches");
        const now = Date.now();
        noteToolCall({ blockId: "ag1", agentName: "korp", color: "#22c55e" }, { id: "t1", tool: "Write", params: { file_path: `${HOME}/b.txt` } });
        noteToolResult("t1", "success", now);
        const badges = () => [...v.container.querySelectorAll(".files-touch")].map((e) => e.getAttribute("aria-label"));
        log(`badges: ${fmt(badges())}`);
        vi.spyOn(Date, "now").mockReturnValue(now + 31 * 60_000);
        tick!();
        log(`after 31 minutes: ${fmt(badges())}`);
        resetTouchedForTests();
        await expect(out()).toMatchFileSnapshot(snap("preview-touch"));
    });
});

describe("FilesView notices and corners", () => {
    const notice = (v: Mounted) => v.q(".files-notice")?.textContent ?? "(none)";

    it("shows the notice for each state of the listing, and what its buttons do", async () => {
        let armed = false;
        const phase = (name: string, over: Record<string, unknown> = {}) => {
            const v = mount({}, (m) => {
                vi.spyOn(m, "phase").mockReturnValue(name as any);
                for (const [k, val] of Object.entries(over)) vi.spyOn(m as any, k).mockReturnValue(val);
                vi.spyOn(m, "navigate").mockImplementation(async (...a) => void (armed && log(`model.navigate(${a.map(fmt).join(", ")})`)));
                vi.spyOn(m, "refresh").mockImplementation(() => void (armed && log("model.refresh()")));
            });
            return v;
        };
        let v = phase("gated", { protectedPlace: { label: "Documents" } });
        log(`gated: ${notice(v)}`);
        armed = true;
        fireEvent.click(v.q(".files-notice button"));
        armed = false;
        cleanup();
        v = phase("loading", { slow: false });
        log(`loading: ${notice(v)}`);
        cleanup();
        v = phase("loading", { slow: true });
        log(`loading slow: ${notice(v)}`);
        cleanup();
        v = phase("loading", { slow: true, protectedPlace: { label: "Documents" } });
        log(`loading slow, protected: ${notice(v)}`);
        cleanup();
        v = phase("error", { error: { kind: "not_found", message: "gone" } });
        log(`error: ${notice(v)}`);
        armed = true;
        fireEvent.click(v.q(".files-notice button"));
        armed = false;
        cleanup();
        h.state.dirs.set(HOME, []);
        v = mount();
        await waitFor(() => expect(v.q(".files-notice")).toBeTruthy());
        log(`empty: ${notice(v)}`);
        v.model.setFilter("zz");
        log(`empty filtered: ${notice(v)}`);
        cleanup();
        h.state.dirs.set(HOME, [entry(".hidden", { hidden: true })]);
        v = mount();
        await waitFor(() => expect(v.q(".files-notice")).toBeTruthy());
        log(`only hidden: ${notice(v)}`);
        await expect(out()).toMatchFileSnapshot(snap("notices"));
    });

    it("refuses a new tab in a confined folder, reports a failed one, copies one path, ignores a drop on itself and focuses the list on request", async () => {
        const writes: string[] = [];
        Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async (t: string) => void writes.push(t) } });
        const v = mount({ "files:root": HOME, "files:path": HOME });
        await settled(v);
        T.length = 0;
        watch(v.model, ["setStatus"]);
        h.state.runner!("files:newTabHere");
        cleanup();
        const w = mount();
        await settled(w);
        watch(w.model, ["setStatus"]);
        stub(w.model, ["transfer"], Promise.resolve());
        h.state.fail = true;
        h.state.runner!("files:newTabHere");
        await flush();
        h.state.fail = false;
        h.state.menuHandlers = {};
        fireEvent.contextMenu(row(w, "b.txt"));
        h.state.menuHandlers.copyPaths([entry("b.txt")]);
        await flush();
        log(`clipboard: ${fmt(writes)}`);
        h.state.drag = { paths: ["/other/a"], source: "b2" };
        await h.state.drop.drop({ paths: ["/home/a"] });
        log("self drop: no transfer above");
        w.model.focusList!();
        log(`focusList: ${document.activeElement === w.list()}`);
        await expect(out()).toMatchFileSnapshot(snap("corners"));
    });

    it("hands the preview its open callbacks, uses the command key as the modifier on a Mac, and renames a tile in place", async () => {
        const v = mount({ "files:preview": true });
        await settled(v);
        T.length = 0;
        select(v, "b.txt");
        const p = h.state.previewProps;
        h.state.target = "editor";
        p.onOpen(entry("b.txt"));
        p.onOpenWithOs(entry("b.txt"));
        log(`pathOf: ${p.pathOf("b.txt")} blockId=${p.blockId}`);
        cleanup();
        setPlatform("darwin");
        const m = mount();
        await settled(m);
        fireEvent.click(row(m, "src"));
        fireEvent.click(row(m, "pic.png"), { metaKey: true });
        log(`mac meta-click: ${sel(m)}`);
        fireEvent.click(row(m, "a.md"), { ctrlKey: true });
        log(`mac ctrl-click: ${sel(m)}`);
        key(m.list(), "ArrowDown", { metaKey: true });
        log(`mac meta+ArrowDown: ${sel(m)}`);
        cleanup();
        setPlatform("linux");
        const g = mount({ "files:view": "grid" });
        await settled(g);
        g.model.setSelection({ names: new Set(["a.md"]), focus: "a.md", anchor: "a.md" });
        g.model.setRenaming("a.md");
        const box = g.q(".files-tile input") as HTMLInputElement;
        log(`tile rename box: ${!!box} value=${box?.value}`);
        key(box, "Escape");
        await flush();
        log(`after Escape: box=${!!g.q(".files-tile input")} renaming=${g.model.renaming()} focusOnList=${document.activeElement === g.list()}`);
        await expect(out()).toMatchFileSnapshot(snap("corners2"));
    });
});
