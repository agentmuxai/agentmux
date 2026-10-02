// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane against a fake srv: listing and paging, navigation and
 * history, the keyboard, operations with undo, live updates, the macOS
 * prompt gate, and OpenFiles' selection request.
 * docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §5–§9, §11.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import type { FsEntry } from "@/types/rpc/FsEntry";

const h = vi.hoisted(() => {
    const state = {
        dirs: new Map<string, unknown[]>(),
        errors: new Map<string, { kind: string; message: string }>(),
        pageSize: 0,
        handlers: [] as ((event: unknown) => void)[],
    };
    const rpc = {
        FsPlacesCommand: vi.fn(async () => ({
            home: "C:\\Users\\a",
            sep: "\\",
            places: [
                { id: "home", label: "Home", path: "C:\\Users\\a", kind: "home" },
                { id: "documents", label: "Documents", path: "C:\\Users\\a\\Documents", kind: "known" },
                { id: "c", label: "C:", path: "C:\\", kind: "drive" },
            ],
        })),
        FsListCommand: vi.fn(async (_c: unknown, req: { path: string; cursor?: string }) => {
            const err = state.errors.get(req.path);
            if (err) return { path: req.path, entries: [], error: err };
            const all = state.dirs.get(req.path) ?? [];
            const start = req.cursor ? Number(req.cursor) : 0;
            const size = state.pageSize || all.length || 1;
            const end = start + size;
            return { path: req.path, entries: all.slice(start, end), cursor: end < all.length ? String(end) : undefined };
        }),
        FsWatchCommand: vi.fn(async () => ({ watch_id: "w1" })),
        FsUnwatchCommand: vi.fn(async () => ({})),
        FsRenameCommand: vi.fn(async (_c: unknown, req: { path: string; new_name: string }) => ({
            new_path: req.path.replace(/[^\\]+$/, req.new_name),
        })),
        FsCreateCommand: vi.fn(async (_c: unknown, req: { parent: string; name: string }) => ({ path: `${req.parent}\\${req.name}` })),
        FsTrashCommand: vi.fn(async (_c: unknown, req: { paths: string[] }) => ({ results: req.paths.map((path) => ({ path, ok: true })) })),
        FsRestoreCommand: vi.fn(async (_c: unknown, req: { paths: string[] }) => ({ results: req.paths.map((path) => ({ path, ok: true })) })),
        FsDeleteCommand: vi.fn(async (_c: unknown, req: { paths: string[] }) => ({ results: req.paths.map((path) => ({ path, ok: true })) })),
        FsOpenCommand: vi.fn(async () => ({})),
        FsRevealCommand: vi.fn(async () => ({})),
        ListNamedAgentsCommand: vi.fn(async () => [
            { instance_name: "korp", definition_name: "korp", definition_id: "def-korp", working_directory: "C:\\Users\\a\\.agentmux\\agents\\korp" },
            { instance_name: "loap", definition_name: "loap", definition_id: "def-loap", working_directory: "C:\\Users\\a\\.agentmux\\agents\\loap" },
        ]),
        GetAgentContentCommand: vi.fn(async (_c: unknown, req: { agent_id: string }) =>
            req.agent_id === "def-korp" ? { agent_id: req.agent_id, content_type: "ui:color", content: "#22c55e", updated_at: 0 } : null
        ),
    };
    const rpcCall = vi.fn(async () => ({}));
    return { state, rpc, rpcCall };
});

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: h.rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: { rpcCall: h.rpcCall } }));
vi.mock("@/app/store/mps", () => ({
    muxEventSubscribe: (sub: { handler: (e: unknown) => void }) => {
        h.state.handlers.push(sub.handler);
        return () => {};
    },
}));

import { getPaneTab } from "@/app/block/pane-tab-registry";
import { setPlatform } from "@/util/platformutil";
import { FilesModel } from "./files-model";
import { openTargetOf } from "./files-open";
import { errorMessage, FilesView, formatModified } from "./files-view";
import { filesPaneTab, filesTitle } from "./files";

const f = (name: string, over: Partial<FsEntry> = {}): FsEntry => ({
    name,
    is_dir: false,
    is_symlink: false,
    hidden: false,
    readonly: false,
    size: 10,
    mtime: 0,
    ...over,
});
const d = (name: string, over: Partial<FsEntry> = {}): FsEntry => f(name, { is_dir: true, size: undefined, ...over });

const HOME = "C:\\Users\\a";

/** `frameContextMenu` stands in for the pane frame, which listens above the view
 *  with a Solid handler (so it takes part in Solid's delegated bubbling). */
function mount(meta: Record<string, unknown> = {}, frameContextMenu?: (e: MouseEvent) => void) {
    const [m, setM] = createSignal<Record<string, unknown>>(meta);
    const [visibility, setVisibility] = createSignal<"active" | "dormant" | "windowHidden">("active");
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
        visibility,
    };
    const model = new FilesModel(ctx);
    const r = render(() => (
        <div onContextMenu={frameContextMenu}>
            <FilesView model={model} ctx={ctx} />
        </div>
    ));
    const names = () => [...r.container.querySelectorAll(".files-rows .files-name")].map((n) => n.textContent);
    const list = () => r.container.querySelector(".files-list") as HTMLElement;
    const row = (name: string) =>
        [...r.container.querySelectorAll(".files-rows .files-row")].find((x) => x.querySelector(".files-name")?.textContent === name) as HTMLElement;
    return { ...r, model, ctx, meta: m, setVisibility, names, list, row };
}

beforeEach(() => {
    setPlatform("win32");
    h.state.dirs.clear();
    h.state.errors.clear();
    h.state.pageSize = 0;
    h.state.handlers = [];
    h.state.dirs.set(HOME, [f("b.txt"), d("src"), f(".env", { hidden: true }), f("a10.md"), f("a2.md")]);
    h.state.dirs.set(`${HOME}\\src`, [f("main.rs")]);
    for (const fn of Object.values(h.rpc)) fn.mockClear();
    h.rpcCall.mockClear();
    // ResizeObserver isn't in jsdom.
    (globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class {
        observe() {}
        disconnect() {}
    };
});

afterEach(() => {
    cleanup();
    setPlatform("darwin");
});

describe("the Files pane: listing", () => {
    it("opens Home, folders first, names sorted naturally, hidden files hidden", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toEqual(["src", "a2.md", "a10.md", "b.txt"]));
        expect(v.meta()["files:path"]).toBe(HOME);
        expect(h.rpc.FsWatchCommand).toHaveBeenCalledWith(expect.anything(), { path: HOME, block_id: "b1" });
        expect(v.container.querySelector(".files-status")?.textContent).toBe("4 items");
    });

    it("shows hidden files when asked, and remembers it in the block", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        fireEvent.click(v.container.querySelector('[title="Show hidden files"]')!);
        await waitFor(() => expect(v.names()).toContain(".env"));
        expect(v.meta()["files:hidden"]).toBe(true);
    });

    it("sorts by a column, and the other way on a second click", async () => {
        h.state.dirs.set(HOME, [f("small", { size: 1 }), f("big", { size: 9 }), f("mid", { size: 5 })]);
        const v = mount();
        await waitFor(() => expect(v.names()).toEqual(["big", "mid", "small"]));
        const size = [...v.container.querySelectorAll(".files-header .files-cell")].find((c) => c.textContent?.startsWith("Size"))!;
        fireEvent.click(size);
        await waitFor(() => expect(v.names()).toEqual(["small", "mid", "big"]));
        fireEvent.click(size);
        await waitFor(() => expect(v.names()).toEqual(["big", "mid", "small"]));
        expect(v.meta()["files:sortdir"]).toBe("desc");
    });

    it("paints the first page, then the rest", async () => {
        h.state.pageSize = 2;
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        expect(h.rpc.FsListCommand.mock.calls.filter((c) => (c[1] as { path: string }).path === HOME).length).toBe(3);
    });

    it("says why a folder can't be listed, never shows it as empty", async () => {
        h.state.errors.set(HOME, { kind: "permission_denied", message: "Access is denied." });
        const v = mount();
        await waitFor(() => expect(v.container.querySelector(".files-notice-error")?.textContent).toContain("You don't have permission"));
        expect(v.container.textContent).not.toContain("This folder is empty");
        h.state.errors.clear();
        fireEvent.click(v.getByText("Try again"));
        await waitFor(() => expect(v.names()).toHaveLength(4));
    });

    it("explains a macOS denial with where to turn access on", () => {
        expect(errorMessage("os_blocked", "Operation not permitted", "Documents")).toBe(
            "macOS blocked AgentMux from reading Documents. Open System Settings → Privacy & Security → Files & Folders, turn AgentMux on, then try again."
        );
    });
});

describe("the Files pane: navigation", () => {
    it("opens a folder on double-click, and goes back, forward and up", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        fireEvent.dblClick(v.row("src"));
        await waitFor(() => expect(v.names()).toEqual(["main.rs"]));
        expect(v.meta()["files:path"]).toBe(`${HOME}\\src`);
        fireEvent.keyDown(v.list(), { key: "ArrowLeft", altKey: true });
        await waitFor(() => expect(v.names()).toHaveLength(4));
        fireEvent.keyDown(v.list(), { key: "ArrowRight", altKey: true });
        await waitFor(() => expect(v.names()).toEqual(["main.rs"]));
        fireEvent.keyDown(v.list(), { key: "ArrowUp", altKey: true });
        await waitFor(() => expect(v.names()).toHaveLength(4));
        // Up lands on the folder it came out of.
        await waitFor(() => expect(v.model.selection().focus).toBe("src"));
    });

    it("opens a file by its kind, beside the pane", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        fireEvent.dblClick(v.row("b.txt"));
        expect(h.rpcCall).toHaveBeenCalledWith(
            "pane.open",
            { view: "editor", file: `${HOME}\\b.txt`, split_direction: "right", split_reference_block_id: "b1" },
            {}
        );
        expect(openTargetOf("a.png")).toBe("media");
        expect(openTargetOf("a.pdf")).toBe("os");
        expect(openTargetOf("setup.exe")).toBe("program");
        expect(openTargetOf("Makefile")).toBe("editor");
    });

    it("asks before running a program", async () => {
        h.state.dirs.set(HOME, [f("setup.exe")]);
        const v = mount();
        await waitFor(() => expect(v.names()).toEqual(["setup.exe"]));
        fireEvent.dblClick(v.row("setup.exe"));
        expect(h.rpc.FsOpenCommand).not.toHaveBeenCalled();
        fireEvent.click(screen.getByText("Run"));
        expect(h.rpc.FsOpenCommand).toHaveBeenCalledWith(expect.anything(), { path: `${HOME}\\setup.exe` });
    });

    it("shows each agent's name in the color its pane border uses", async () => {
        const v = mount();
        await waitFor(() => expect(v.getByText("korp")).toBeTruthy());
        // A stored ui:color wins; an agent with none gets the deterministic
        // pick its pane would be seeded with.
        expect((v.getByText("korp") as HTMLElement).style.color).toBe("rgb(34, 197, 94)");
        const loap = (v.getByText("loap") as HTMLElement).style.color;
        expect(loap).not.toBe("");
        expect(loap).not.toBe("rgb(34, 197, 94)");
    });

    it("opens only Hangar's menu on blank space, not the pane frame's as well", async () => {
        // The pane frame listens above the view; a right-click that reaches it
        // opens the generic pane menu on top of Hangar's own.
        const reachedFrame = vi.fn();
        const v = mount({}, reachedFrame);
        await waitFor(() => expect(v.names()).toHaveLength(4));
        fireEvent.contextMenu(v.list());
        expect(document.body.querySelector(".ctx-menu")).toBeTruthy();
        expect(reachedFrame).not.toHaveBeenCalled();
        // Rows already behaved this way.
        fireEvent.keyDown(document.body, { key: "Escape" });
        fireEvent.contextMenu(v.row("src"));
        expect(reachedFrame).not.toHaveBeenCalled();
    });

    it("lists each agent's folder under Places", async () => {
        const v = mount();
        await waitFor(() => expect(v.getByText("korp")).toBeTruthy());
        fireEvent.click(v.getByText("korp"));
        await waitFor(() => expect(v.meta()["files:path"]).toBe(`${HOME}\\.agentmux\\agents\\korp`));
    });
});

describe("the Files pane: keyboard and selection", () => {
    it("moves with the arrows, extends with Shift, selects all, and jumps by typing", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        fireEvent.keyDown(v.list(), { key: "ArrowDown" });
        expect([...v.model.selection().names]).toEqual(["src"]);
        fireEvent.keyDown(v.list(), { key: "ArrowDown", shiftKey: true });
        expect([...v.model.selection().names]).toEqual(["src", "a2.md"]);
        expect(v.container.querySelector(".files-status")?.textContent).toContain("2 selected");
        fireEvent.keyDown(v.list(), { key: "a", ctrlKey: true });
        expect(v.model.selection().names.size).toBe(4);
        fireEvent.keyDown(v.list(), { key: "b" });
        expect([...v.model.selection().names]).toEqual(["b.txt"]);
    });

    it("selects with clicks: plain, Ctrl and Shift", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        fireEvent.click(v.row("a2.md"));
        fireEvent.click(v.row("b.txt"), { shiftKey: true });
        expect([...v.model.selection().names]).toEqual(["a2.md", "a10.md", "b.txt"]);
        fireEvent.click(v.row("a10.md"), { ctrlKey: true });
        expect([...v.model.selection().names]).toEqual(["a2.md", "b.txt"]);
    });
});

describe("the Files pane: operations", () => {
    it("Delete moves the selection to the Trash, and Undo restores it", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        fireEvent.click(v.row("b.txt"));
        fireEvent.keyDown(v.list(), { key: "Delete" });
        await waitFor(() => expect(h.rpc.FsTrashCommand).toHaveBeenCalledWith(expect.anything(), { paths: [`${HOME}\\b.txt`] }));
        await waitFor(() => expect(v.container.querySelector(".files-status")?.textContent).toContain("Moved b.txt to Trash"));
        fireEvent.click(v.getByText("Undo"));
        await waitFor(() => expect(h.rpc.FsRestoreCommand).toHaveBeenCalledWith(expect.anything(), { paths: [`${HOME}\\b.txt`] }));
    });

    it("Shift+Delete asks, naming the item, before deleting for good", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        fireEvent.click(v.row("src"));
        fireEvent.keyDown(v.list(), { key: "Delete", shiftKey: true });
        expect(screen.getByText('Delete "src" permanently?')).toBeTruthy();
        expect(v.container.ownerDocument.body.textContent).toContain("Folders are deleted with everything in them.");
        expect(h.rpc.FsDeleteCommand).not.toHaveBeenCalled();
        fireEvent.click(screen.getByText("Delete permanently"));
        await waitFor(() => expect(h.rpc.FsDeleteCommand).toHaveBeenCalledWith(expect.anything(), { paths: [`${HOME}\\src`] }));
    });

    it("F2 renames in place, checking the name first", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        fireEvent.click(v.row("b.txt"));
        fireEvent.keyDown(v.list(), { key: "F2" });
        const input = v.container.querySelector(".files-rename-input") as HTMLInputElement;
        expect(input).not.toBeNull();
        // The stem is selected, not the extension.
        expect([input.selectionStart, input.selectionEnd]).toEqual([0, 1]);
        input.value = "CON";
        fireEvent.keyDown(input, { key: "Enter" });
        expect(v.container.querySelector(".files-rename-problem")?.textContent).toBe('"CON" is reserved by Windows.');
        expect(h.rpc.FsRenameCommand).not.toHaveBeenCalled();
        input.value = "c.txt";
        fireEvent.keyDown(input, { key: "Enter" });
        await waitFor(() => expect(h.rpc.FsRenameCommand).toHaveBeenCalledWith(expect.anything(), { path: `${HOME}\\b.txt`, new_name: "c.txt" }));
    });

    it("New folder picks a free name and starts renaming it", async () => {
        h.state.dirs.set(HOME, [d("New folder")]);
        const v = mount();
        await waitFor(() => expect(v.names()).toEqual(["New folder"]));
        fireEvent.click(v.container.querySelector('[title="New folder"]')!);
        await waitFor(() =>
            expect(h.rpc.FsCreateCommand).toHaveBeenCalledWith(expect.anything(), { parent: HOME, name: "New folder (2)", kind: "dir" })
        );
    });
});

describe("the Files pane: rows (ReAgent on #4201)", () => {
    it("keeps each row's element across a scroll and a re-list", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        const before = v.row("b.txt");
        fireEvent.scroll(v.list(), { target: { scrollTop: 5 } });
        expect(v.row("b.txt")).toBe(before);
        h.state.dirs.set(HOME, [f("b.txt"), d("src"), f("a2.md"), f("a10.md"), f("z.txt")]);
        h.state.handlers.forEach((fn) => fn({ data: { dir: HOME } }));
        await waitFor(() => expect(v.names()).toContain("z.txt"));
        expect(v.row("b.txt")).toBe(before);
    });

    it("commits a rename once: the blur after Enter doesn't repeat it", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        fireEvent.click(v.row("b.txt"));
        fireEvent.keyDown(v.list(), { key: "F2" });
        const input = v.container.querySelector(".files-rename-input") as HTMLInputElement;
        input.value = "c.txt";
        fireEvent.keyDown(input, { key: "Enter" });
        fireEvent.blur(input);
        await waitFor(() => expect(h.rpc.FsRenameCommand).toHaveBeenCalledTimes(1));
        await new Promise((r) => setTimeout(r, 20));
        expect(h.rpc.FsRenameCommand).toHaveBeenCalledTimes(1);
    });

    it("Escape cancels: the blur that follows doesn't rename", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        fireEvent.click(v.row("b.txt"));
        fireEvent.keyDown(v.list(), { key: "F2" });
        const input = v.container.querySelector(".files-rename-input") as HTMLInputElement;
        input.value = "c.txt";
        fireEvent.keyDown(input, { key: "Escape" });
        fireEvent.blur(input);
        await new Promise((r) => setTimeout(r, 20));
        expect(h.rpc.FsRenameCommand).not.toHaveBeenCalled();
    });
});

describe("the Files pane: scrolling to a row (ReAgent on #4201)", () => {
    const many = () => Array.from({ length: 200 }, (_, i) => f(`file${String(i).padStart(3, "0")}.txt`));

    it("scrolls a new item into view so its rename box mounts", async () => {
        h.state.dirs.set(HOME, many());
        const v = mount();
        await waitFor(() => expect(v.names().length).toBeGreaterThan(0));
        // Lists after the create include the new file, which sorts last.
        h.state.dirs.set(HOME, [...many(), f("New file.txt")]);
        await v.model.createNew("file");
        await waitFor(() => expect(v.container.querySelector(".files-rename-input")).not.toBeNull());
        expect(v.list().scrollTop).toBeGreaterThan(0);
    });

    it("scrolls OpenFiles' selection into view", async () => {
        h.state.dirs.set(HOME, many());
        const v = mount({ "files:path": HOME, "files:select": ["file180.txt"] });
        await waitFor(() => expect(v.model.selection().focus).toBe("file180.txt"));
        await waitFor(() => expect(v.list().scrollTop).toBeGreaterThan(0));
    });
});

describe("the Files pane: deliberate selection (ReAgent on #4201)", () => {
    it("Delete does nothing once the selection is cleared, even with a row focused", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        fireEvent.click(v.row("b.txt"));
        fireEvent.click(v.list());
        expect(v.model.selection().names.size).toBe(0);
        fireEvent.keyDown(v.list(), { key: "Delete" });
        fireEvent.keyDown(v.list(), { key: "Delete", shiftKey: true });
        await new Promise((r) => setTimeout(r, 20));
        expect(h.rpc.FsTrashCommand).not.toHaveBeenCalled();
        expect(screen.queryByText(/permanently\?/)).toBeNull();
    });

    it("leaving the rename box with a bad name cancels it, so the keys work again", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        fireEvent.click(v.row("b.txt"));
        fireEvent.keyDown(v.list(), { key: "F2" });
        const input = v.container.querySelector(".files-rename-input") as HTMLInputElement;
        input.value = "a:b";
        // Focus moves elsewhere: a real blur, with the box no longer active.
        v.list().focus();
        await waitFor(() => expect(v.model.renaming()).toBeNull());
        expect(h.rpc.FsRenameCommand).not.toHaveBeenCalled();
        fireEvent.keyDown(v.list(), { key: "ArrowUp" });
        expect(v.model.selection().focus).toBe("a10.md");
    });

    it("keeps renaming when only the window loses focus (Alt+Tab)", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        fireEvent.click(v.row("b.txt"));
        fireEvent.keyDown(v.list(), { key: "F2" });
        const input = v.container.querySelector(".files-rename-input") as HTMLInputElement;
        expect(document.activeElement).toBe(input);
        input.value = "c.txt";
        // A window blur: the event fires, but the box stays the active element.
        fireEvent.blur(input);
        await new Promise((r) => setTimeout(r, 20));
        expect(v.model.renaming()).toBe("b.txt");
        expect(h.rpc.FsRenameCommand).not.toHaveBeenCalled();
    });

    it("watches again after a failed watch", async () => {
        h.rpc.FsWatchCommand.mockRejectedValueOnce(new Error("cap"));
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        await waitFor(() => expect(h.rpc.FsWatchCommand).toHaveBeenCalledTimes(1));
        v.model.refresh();
        await waitFor(() => expect(h.rpc.FsWatchCommand).toHaveBeenCalledTimes(2));
    });
});

describe("the Files pane: live", () => {
    it("re-lists when srv says the folder changed", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        h.state.dirs.set(HOME, [f("new.txt")]);
        h.state.handlers.forEach((fn) => fn({ data: { dir: HOME } }));
        await waitFor(() => expect(v.names()).toEqual(["new.txt"]));
    });

    it("waits until it's shown to re-list a change made while hidden", async () => {
        const v = mount();
        await waitFor(() => expect(v.names()).toHaveLength(4));
        v.setVisibility("dormant");
        const calls = h.rpc.FsListCommand.mock.calls.length;
        h.state.dirs.set(HOME, [f("new.txt")]);
        h.state.handlers.forEach((fn) => fn({ data: { dir: HOME } }));
        expect(h.rpc.FsListCommand.mock.calls.length).toBe(calls);
        v.setVisibility("active");
        await waitFor(() => expect(v.names()).toEqual(["new.txt"]));
    });

    it("selects what OpenFiles asked for even when the request lands after the listing", async () => {
        const v = mount({ "files:path": HOME });
        await waitFor(() => expect(v.names()).toHaveLength(4));
        await v.ctx.setMeta({ "files:select": ["a2.md"] });
        await waitFor(() => expect([...v.model.selection().names]).toEqual(["a2.md"]));
        expect(v.meta()["files:select"]).toBeUndefined();
    });

    it("selects what OpenFiles asked for, once", async () => {
        const v = mount({ "files:path": HOME, "files:select": ["C:\\Users\\a\\b.txt", "src"] });
        await waitFor(() => expect([...v.model.selection().names].sort()).toEqual(["b.txt", "src"]));
        expect(v.meta()["files:select"]).toBeUndefined();
    });
});

describe("the Files pane: macOS access prompts (§9.1)", () => {
    beforeEach(() => {
        setPlatform("darwin");
        localStorage.clear();
        h.rpc.FsPlacesCommand.mockResolvedValueOnce({
            home: "/Users/a",
            sep: "/",
            places: [
                { id: "home", label: "Home", path: "/Users/a", kind: "home" },
                { id: "documents", label: "Documents", path: "/Users/a/Documents", kind: "known" },
            ],
        });
        h.state.dirs.set("/Users/a/Documents", [f("cv.pdf")]);
    });

    it("never lists a protected folder on restore: it waits for a click", async () => {
        const v = mount({ "files:path": "/Users/a/Documents" });
        await waitFor(() => expect(v.container.querySelector(".files-notice")?.textContent).toContain("macOS will ask to let AgentMux open your Documents folder"));
        expect(h.rpc.FsListCommand).not.toHaveBeenCalled();
        fireEvent.click(v.getByText("Open Documents"));
        await waitFor(() => expect(v.names()).toEqual(["cv.pdf"]));
    });

    it("explains before the prompt when the user clicks it in Places, too", async () => {
        h.state.dirs.set("/Users/a", [d("Documents")]);
        const v = mount();
        await waitFor(() => expect(v.names()).toEqual(["Documents"]));
        fireEvent.click(v.getByText("Documents", { selector: ".files-place span" }));
        await waitFor(() => expect(v.getByText("Open Documents")).toBeTruthy());
        expect(h.rpc.FsListCommand.mock.calls.some((c) => (c[1] as { path: string }).path === "/Users/a/Documents")).toBe(false);
        fireEvent.click(v.getByText("Open Documents"));
        await waitFor(() => expect(v.names()).toEqual(["cv.pdf"]));
    });

    it("gates a protected folder however its path is spelled (ReAgent on #4201)", async () => {
        h.state.dirs.set("/Users/a", [d("Documents")]);
        const v = mount({ "files:path": "/Users/a/Desktop/../Documents" });
        await waitFor(() => expect(v.getByText("Open Documents")).toBeTruthy());
        expect(h.rpc.FsListCommand).not.toHaveBeenCalled();
        cleanup();
        h.rpc.FsPlacesCommand.mockResolvedValueOnce({
            home: "/Users/a",
            sep: "/",
            places: [
                { id: "home", label: "Home", path: "/Users/a", kind: "home" },
                { id: "documents", label: "Documents", path: "/Users/a/Documents", kind: "known" },
            ],
        });
        const tilde = mount({ "files:path": "~/Documents" });
        await waitFor(() => expect(tilde.getByText("Open Documents")).toBeTruthy());
        expect(h.rpc.FsListCommand).not.toHaveBeenCalled();
    });

    it("Refresh, F5 and New folder don't list a gated folder (ReAgent on #4201)", async () => {
        const v = mount({ "files:path": "/Users/a/Documents" });
        await waitFor(() => expect(v.getByText("Open Documents")).toBeTruthy());
        fireEvent.click(v.container.querySelector('[title="Refresh (F5)"]')!);
        fireEvent.keyDown(v.list(), { key: "F5" });
        fireEvent.click(v.container.querySelector('[title="New folder"]')!);
        await new Promise((r) => setTimeout(r, 20));
        expect(h.rpc.FsListCommand).not.toHaveBeenCalled();
        expect(h.rpc.FsCreateCommand).not.toHaveBeenCalled();
        expect(v.getByText("Open Documents")).toBeTruthy();
    });

    it("Try again works after macOS denies the folder (ReAgent on #4201)", async () => {
        h.state.errors.set("/Users/a/Documents", { kind: "os_blocked", message: "Operation not permitted" });
        const v = mount({ "files:path": "/Users/a/Documents" });
        await waitFor(() => expect(v.getByText("Open Documents")).toBeTruthy());
        fireEvent.click(v.getByText("Open Documents"));
        await waitFor(() => expect(v.container.querySelector(".files-notice-error")?.textContent).toContain("macOS blocked AgentMux"));
        h.state.errors.clear();
        fireEvent.click(v.getByText("Try again"));
        await waitFor(() => expect(v.names()).toEqual(["cv.pdf"]));
    });

    it("ignores case, as APFS does (ReAgent on #4201)", async () => {
        const v = mount({ "files:path": "~/documents" });
        await waitFor(() => expect(v.getByText("Open Documents")).toBeTruthy());
        expect(h.rpc.FsListCommand).not.toHaveBeenCalled();
    });

    it("lists it straight away once this release has opened it", async () => {
        const first = mount({ "files:path": "/Users/a/Documents" });
        await waitFor(() => expect(first.getByText("Open Documents")).toBeTruthy());
        fireEvent.click(first.getByText("Open Documents"));
        await waitFor(() => expect(first.names()).toEqual(["cv.pdf"]));
        cleanup();
        h.rpc.FsPlacesCommand.mockResolvedValueOnce({
            home: "/Users/a",
            sep: "/",
            places: [{ id: "documents", label: "Documents", path: "/Users/a/Documents", kind: "known" }],
        });
        const again = mount({ "files:path": "/Users/a/Documents" });
        await waitFor(() => expect(again.names()).toEqual(["cv.pdf"]));
    });
});

describe("the Files pane: registration", () => {
    it("registers as a keep-alive pane tab titled by its folder", () => {
        expect(filesPaneTab).toMatchObject({ view: "files", label: "Hangar", icon: "folder-open" });
        expect(filesPaneTab.capabilities?.lifecycle).toBe("keepAlive");
        expect(filesTitle({ "files:path": "C:\\Users\\a\\src" } as MetaType)).toBe("src");
        expect(filesTitle(undefined)).toBe("Hangar");
        void getPaneTab;
    });

    it("formats modified times", () => {
        const now = Date.UTC(2026, 9, 1, 12);
        expect(formatModified(now - 10_000, now)).toBe("just now");
        expect(formatModified(now - 5 * 60_000, now)).toBe("5 min ago");
        expect(formatModified(now - 3 * 3_600_000, now)).toBe("3 h ago");
        expect(formatModified(now - 30 * 3_600_000, now)).toBe("yesterday");
        expect(formatModified(undefined, now)).toBe("");
    });
});
