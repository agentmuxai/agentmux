// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * FileTree (the editor's file explorer), pinned as a whole ahead of splitting it into a state object and smaller
 * views: each scenario mounts the real tree over the real FileTreeModel (only the directory listing RPC and the
 * toolbar button are stand-ins), logs every model call it makes and every callback it fires, drives its props,
 * clicks, types and presses keys, and logs the tree as an indented tree (attributes included) and, in order, every
 * call. The logs are snapshots recorded on the unsplit component; the split must leave them byte for byte.
 * (file-tree.test.tsx keeps the keyboard and reveal checks it was written for.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({
    lists: new Map<string, any>(),
    pending: new Map<string, (v: any) => void>(),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListEditorDirCommand: async (_c: unknown, req: { path: string }) => {
            log(`ListEditorDirCommand ${req.path}`);
            const v = h.lists.get(req.path);
            if (v === "slow") return new Promise((res) => h.pending.set(req.path, res));
            if (v === "broken") throw new Error(`cannot read ${req.path}`);
            return { path: req.path, entries: v ?? [] };
        },
    },
}));
vi.mock("@/app/element/ui", () => ({
    IconButton: (p: {
        icon: string;
        label: string;
        density?: string;
        pressed?: boolean;
        onClick: () => void;
        children?: JSX.Element;
    }) => (
        <span
            data-stand="IconButton"
            data-icon={p.icon}
            data-label={p.label}
            data-density={p.density}
            data-pressed={String(p.pressed)}
            onClick={() => (log(`IconButton ${p.label} onClick`), p.onClick())}
        />
    ),
}));

import { FileTree } from "./file-tree";
import { FileTreeModel } from "./file-tree-model";

const SNAP = (name: string) => `./__snapshots__/file-tree.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|value|placeholder|tabindex)$/;

/** The tree as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    const attrs = [...el.attributes]
        .filter((a) => ATTRS.test(a.name))
        .map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

const entry = (name: string, is_dir = false, is_symlink = false) => ({ name, is_dir, is_symlink });
const HOME = "/home/a";

type Props = {
    activeFilePath: string;
    showHidden: boolean;
    renamingPath: string | null;
    newEntry: { parentPath: string; kind: "file" | "dir" } | null;
};
let model: FileTreeModel;
let host: HTMLElement;
let unmount: () => void;
let setProps: (p: Partial<Props>) => void;

const flush = async () => {
    for (let i = 0; i < 12; i++) await Promise.resolve();
};
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const tree = () => host.querySelector<HTMLElement>("[role=tree]")!;
const item = (path: string) => host.querySelector<HTMLElement>(`[role=treeitem][data-path="${path}"]`)!;
const rowOf = (path: string) => item(path).querySelector<HTMLElement>(":scope > .file-tree-row")!;
const where = () => {
    const a = document.activeElement as HTMLElement | null;
    return a?.dataset?.path ? `focus on ${a.dataset.path}` : a?.tagName === "INPUT" ? "focus in an input" : "focus elsewhere";
};
const press = (key: string, init: KeyboardEventInit = {}) => {
    const ok = fireEvent.keyDown(tree(), { key, ...init });
    log(`${key}${init.ctrlKey ? " +ctrl" : ""}: prevented=${!ok} selected=${model.selectedPath()} ${where()}`);
};

/** Spy on the model: each call is logged, then runs for real. */
function watch(m: FileTreeModel) {
    for (const name of ["setSelectedPath", "toggleExpand", "expandFolder", "collapseFolder", "collapseAll", "refresh", "revealPath"] as const) {
        const orig = (m as any)[name].bind(m);
        (m as any)[name] = (...a: any[]) => (log(`model.${name}${a.length ? " " + a.map(String).join(" ") : ""}`), orig(...a));
    }
}

async function mount(
    init: Partial<Props> = {},
    opts: { withMenu?: boolean; withEditor?: boolean; withRename?: boolean; withHide?: boolean; open?: string[] } = {}
) {
    const logged = T.length;
    h.lists.clear();
    h.pending.clear();
    h.lists.set(HOME, [
        entry("docs", true),
        entry("src", true),
        entry(".git", true),
        entry("node_modules", true),
        entry("slow", true),
        entry("broken", true),
        entry("link", false, true),
        entry(".env"),
        entry("README.md"),
    ]);
    h.lists.set(`${HOME}/src`, [entry("main.ts"), entry("util.ts"), entry("zeta.ts"), entry("deep", true)]);
    h.lists.set(`${HOME}/src/deep`, [entry("leaf.ts")]);
    h.lists.set(`${HOME}/docs`, [entry("guide.md"), entry("notes.txt")]);
    h.lists.set(`${HOME}/slow`, "slow");
    h.lists.set(`${HOME}/broken`, "broken");
    model = new FileTreeModel();
    await model.setRootsAndLoad(HOME, [{ name: "/", path: "/" }]);
    for (const p of opts.open ?? []) await model.expandFolder(p);
    T.length = logged;
    watch(model);
    const [props, set] = createSignal<Props>({
        activeFilePath: "",
        showHidden: false,
        renamingPath: null,
        newEntry: null,
        ...init,
    });
    setProps = (p) => set((prev) => ({ ...prev, ...p }));
    const view = render(() => (
        <FileTree
            model={model}
            activeFilePath={props().activeFilePath}
            showHidden={props().showHidden}
            renamingPath={props().renamingPath}
            newEntry={props().newEntry}
            onFileClick={(p) => log(`onFileClick ${p}`)}
            onFileDblClick={(p) => log(`onFileDblClick ${p}`)}
            onToggleHidden={() => log("onToggleHidden")}
            onHideTree={opts.withHide === false ? undefined : () => log("onHideTree")}
            onContextMenu={
                opts.withMenu === false ? undefined : (p, isDir, e) => log(`onContextMenu ${p} dir=${isDir} x=${e.clientX}`)
            }
            onRenameConfirm={opts.withRename === false ? undefined : (p, n) => log(`onRenameConfirm ${p} ${JSON.stringify(n)}`)}
            onRenameCancel={opts.withRename === false ? undefined : () => log("onRenameCancel")}
            onNewEntryConfirm={(parent, name, kind) => log(`onNewEntryConfirm ${parent} ${JSON.stringify(name)} ${kind}`)}
            onNewEntryCancel={() => log("onNewEntryCancel")}
            onStartRename={opts.withRename === false ? undefined : (p) => log(`onStartRename ${p}`)}
            onFocusEditor={opts.withEditor === false ? undefined : () => log("onFocusEditor")}
        />
    ));
    host = document.body.appendChild(view.container);
    unmount = view.unmount;
    await flush();
}

beforeEach(() => {
    T.length = 0;
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
});
afterEach(() => {
    cleanup();
    document.body.innerHTML = "";
    vi.useRealTimers();
});

describe("FileTree, pinned", () => {
    it("lists the roots and the open folders with their place, level and state, hides dotfiles and node_modules unless asked, marks symlinks, and shows a loading and an error line", async () => {
        await mount({}, { open: [`${HOME}/src`, `${HOME}/docs`] });
        look("home open, src and docs open");
        void model.toggleExpand(`${HOME}/slow`);
        void model.toggleExpand(`${HOME}/broken`);
        await flush();
        look("a folder still loading and one that failed");
        setProps({ showHidden: true });
        look("hidden files shown");
        setProps({ showHidden: false });
        model.collapseAll();
        look("collapsed to the roots");
        return snapshot("structure");
    });

    it("has a toolbar whose buttons hide the tree (only when it can be hidden), toggle hidden files with their state, collapse all and refresh", async () => {
        await mount({}, { open: [`${HOME}/src`] });
        look("toolbar with hide");
        const btn = (label: string) => host.querySelector<HTMLElement>(`[data-label="${label}"]`)!;
        fireEvent.click(btn("Hide file tree"));
        fireEvent.click(btn("Show hidden files"));
        setProps({ showHidden: true });
        fireEvent.click(btn("Hide hidden files"));
        fireEvent.click(btn("Collapse all folders"));
        fireEvent.click(btn("Refresh tree"));
        await flush();
        cleanup();
        await mount({}, { withHide: false });
        look("toolbar without hide");
        return snapshot("toolbar");
    });

    it("previews a file on click, keeps it on double-click, toggles a folder on click (ignoring double-click), hands context menus on rows and the background to the owner, and ignores them when there is no owner", async () => {
        await mount({}, { open: [`${HOME}/src`] });
        fireEvent.click(rowOf(`${HOME}/README.md`));
        fireEvent.dblClick(rowOf(`${HOME}/README.md`));
        fireEvent.click(rowOf(`${HOME}/docs`));
        await flush();
        fireEvent.dblClick(rowOf(`${HOME}/docs`));
        fireEvent.click(rowOf(`${HOME}/docs`));
        await flush();
        const ctx = (el: Element, x: number) => {
            const ev = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: x });
            el.dispatchEvent(ev);
            log(`contextmenu prevented=${ev.defaultPrevented}`);
        };
        ctx(rowOf(`${HOME}/src/main.ts`), 11);
        ctx(rowOf(`${HOME}/src`), 12);
        ctx(tree(), 13);
        cleanup();
        await mount({}, { withMenu: false, open: [`${HOME}/src`] });
        ctx(rowOf(`${HOME}/src/main.ts`), 21);
        ctx(tree(), 22);
        fireEvent.focusIn(item(`${HOME}/src/util.ts`));
        look("after focusing a row");
        return snapshot("clicks");
    });

    it("moves with the arrow, Home, End and page keys, opens and closes folders with the side arrows, and previews a file only after a pause", async () => {
        await mount({}, { open: [`${HOME}/src`, `${HOME}/docs`] });
        const selectThen = (path: string, ...keys: string[]) => {
            model.setSelectedPath(path);
            keys.forEach((k) => press(k));
        };
        selectThen(`${HOME}/src`, "ArrowLeft", "ArrowRight", "ArrowRight", "ArrowLeft", "ArrowLeft", "ArrowLeft");
        selectThen(HOME, "ArrowRight", "ArrowRight", "ArrowRight", "ArrowLeft");
        selectThen(`${HOME}/README.md`, "ArrowRight", "ArrowDown", "ArrowDown");
        for (const k of ["End", "Home", "PageDown", "PageUp"]) press(k);
        selectThen(HOME, "ArrowDown");
        press("ArrowDown");
        vi.advanceTimersByTime(60);
        log("(60 ms later: no preview yet)");
        press("ArrowDown");
        vi.advanceTimersByTime(99);
        log("(99 ms: no preview yet)");
        vi.advanceTimersByTime(2);
        log("(a pause: preview)");
        press("ArrowDown");
        press("ArrowUp");
        vi.advanceTimersByTime(500);
        look("after moving about");
        return snapshot("keys-move");
    });

    it("opens with Enter and Space, renames with F2 (never a root), leaves for the editor with Escape, types ahead, and leaves modified keys and already-handled keys alone", async () => {
        await mount({}, { open: [`${HOME}/src`] });
        model.setSelectedPath(`${HOME}/src/main.ts`);
        press("Enter");
        press(" ");
        model.setSelectedPath(`${HOME}/docs`);
        press("Enter");
        press(" ");
        model.setSelectedPath(`${HOME}/src/util.ts`);
        press("F2");
        model.setSelectedPath(HOME);
        press("F2");
        press("Escape");
        press("u");
        press("t");
        press("a", { ctrlKey: true });
        press("x", { metaKey: true });
        press("Tab");
        press("Shift");
        const pre = new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true });
        pre.preventDefault();
        tree().dispatchEvent(pre);
        log(`already prevented: selected=${model.selectedPath()}`);
        cleanup();
        await mount({}, { withEditor: false, withRename: false, open: [`${HOME}/src`] });
        model.setSelectedPath(`${HOME}/src/main.ts`);
        press("Enter");
        press("F2");
        press("Escape");
        press(" ");
        return snapshot("keys-act");
    });

    it("lets a space belong to the name being typed once type-ahead is going, and with nothing selected acts on the first row", async () => {
        await mount({}, { open: [`${HOME}/src`] });
        model.setSelectedPath(HOME);
        press("m");
        press(" ");
        model.collapseAll();
        model.setSelectedPath(null);
        await flush();
        vi.advanceTimersByTime(5000);
        press(" ");
        press("Enter");
        press("F2");
        return snapshot("keys-space");
    });

    it("renames a row in place: the input takes focus with its name selected, Enter confirms the trimmed name, Escape or an empty name cancels, a blur confirms, and nothing fires twice", async () => {
        await mount({ renamingPath: `${HOME}/README.md` });
        look("renaming README.md");
        const input = () => host.querySelector<HTMLInputElement>("input")!;
        log(`${where()}; selection ${input().selectionStart}-${input().selectionEnd}`);
        fireEvent.click(rowOf(`${HOME}/README.md`));
        input().value = "  Renamed.md  ";
        fireEvent.input(input());
        fireEvent.keyDown(input(), { key: "x" });
        fireEvent.keyDown(input(), { key: "Enter" });
        fireEvent.blur(input());
        setProps({ renamingPath: null });
        look("renamed");
        setProps({ renamingPath: `${HOME}/README.md` });
        input().value = "   ";
        fireEvent.input(input());
        fireEvent.keyDown(input(), { key: "Enter" });
        setProps({ renamingPath: null });
        setProps({ renamingPath: `${HOME}/docs` });
        fireEvent.keyDown(input(), { key: "Escape" });
        fireEvent.blur(input());
        setProps({ renamingPath: null });
        setProps({ renamingPath: `${HOME}/link` });
        input().value = "other";
        fireEvent.input(input());
        fireEvent.blur(input());
        setProps({ renamingPath: null });
        cleanup();
        await mount({ renamingPath: `${HOME}/README.md` }, { withRename: false });
        fireEvent.keyDown(host.querySelector("input")!, { key: "Escape" });
        host.querySelector("input")!.value = "z";
        fireEvent.input(host.querySelector("input")!);
        fireEvent.blur(host.querySelector("input")!);
        return snapshot("rename");
    });

    it("shows an inline input for a new file or folder inside the open parent, with its icon and placeholder, and confirms or cancels it", async () => {
        await mount({ newEntry: { parentPath: `${HOME}/src`, kind: "file" } }, { open: [`${HOME}/src`] });
        look("new file in src");
        const input = () => host.querySelector<HTMLInputElement>("input")!;
        input().value = " fresh.ts ";
        fireEvent.input(input());
        fireEvent.keyDown(input(), { key: "Enter" });
        setProps({ newEntry: { parentPath: `${HOME}/src`, kind: "dir" } });
        look("new folder in src");
        fireEvent.keyDown(input(), { key: "Escape" });
        setProps({ newEntry: { parentPath: `${HOME}/docs`, kind: "file" } });
        look("parent closed: nothing shown");
        void model.toggleExpand(`${HOME}/docs`);
        await flush();
        look("docs opened: its input appears");
        setProps({ newEntry: null });
        look("cleared");
        return snapshot("new-entry");
    });

    it("marks and reveals the open file: opens its folders, selects its row unless the keyboard is in the tree, scrolls to it, skips a file opened from the tree, and reveals again when the roots arrive", async () => {
        await mount({ activeFilePath: `${HOME}/src/main.ts` });
        await flush();
        look("open file revealed");
        const row = rowOf(`${HOME}/src/util.ts`);
        row.scrollIntoView = () => log("scrollIntoView util.ts");
        rowOf(`${HOME}/src/main.ts`).scrollIntoView = () => log("scrollIntoView main.ts");
        item(`${HOME}/src/util.ts`).focus();
        setProps({ activeFilePath: `${HOME}/src/util.ts` });
        await flush();
        log(`tree has focus: selected stays ${model.selectedPath()}`);
        (document.activeElement as HTMLElement).blur();
        setProps({ activeFilePath: `${HOME}/src/main.ts` });
        await flush();
        look("keyboard outside the tree: selection follows");
        setProps({ activeFilePath: "/elsewhere/else.ts" });
        await flush();
        log(`outside every root: selected ${model.selectedPath()}`);
        setProps({ activeFilePath: "" });
        await flush();
        log("(no open file: nothing to reveal)");
        return snapshot("reveal");
    });

    it("does not reveal a file the user opened from the tree, but reveals the next one, and drops a pending preview when the tree goes away", async () => {
        await mount({ activeFilePath: `${HOME}/README.md` }, { open: [`${HOME}/src`] });
        await flush();
        log("-- open from the tree");
        fireEvent.click(rowOf(`${HOME}/src/zeta.ts`));
        setProps({ activeFilePath: `${HOME}/src/zeta.ts` });
        await flush();
        log("-- open from outside");
        setProps({ activeFilePath: `${HOME}/docs/guide.md` });
        await flush();
        look("revealed");
        log("-- a pending preview");
        model.setSelectedPath(`${HOME}/src`);
        press("ArrowDown");
        unmount();
        vi.advanceTimersByTime(1000);
        log("(unmounted before the pause ended: no preview)");
        return snapshot("opened-here");
    });
});
