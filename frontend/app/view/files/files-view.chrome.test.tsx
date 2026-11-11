// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane's chrome, pinned as markup: the toolbar, Places, column header, progress bars,
 * conflict dialog and status line, in each state they take. The behaviour is in files-view.test.tsx;
 * this is what keeps a refactor of FilesView from changing how the pane looks. Regenerate the
 * committed snapshots with `vitest -u` only for a change meant to look different.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import type { FsEntry } from "@/types/rpc/FsEntry";

const h = vi.hoisted(() => {
    const state = { dirs: new Map<string, unknown[]>(), byType: new Map<string, ((event: unknown) => void)[]>() };
    const rpc = {
        FsPlacesCommand: vi.fn(async () => ({
            home: "/home/a",
            sep: "/",
            places: [
                { id: "home", label: "Home", path: "/home/a", kind: "home" },
                { id: "documents", label: "Documents", path: "/home/a/Documents", kind: "known" },
                { id: "c", label: "C:", path: "C:\\", kind: "drive" },
                { id: "ubuntu", label: "Ubuntu", path: "\\\\wsl$\\Ubuntu", kind: "wsl" },
            ],
        })),
        FsListCommand: vi.fn(async (_c: unknown, req: { path: string }) => ({ path: req.path, entries: state.dirs.get(req.path) ?? [] })),
        FsWatchCommand: vi.fn(async () => ({ watch_id: "w1" })),
        FsUnwatchCommand: vi.fn(async () => ({})),
        FsTrashCommand: vi.fn(async (_c: unknown, req: { paths: string[] }) => ({ results: req.paths.map((path) => ({ path, ok: true })) })),
        FsGitStatusCommand: vi.fn<(client: unknown, req: { path: string }) => Promise<Record<string, unknown>>>(async () => ({ in_repo: false, entries: [], changes: 0 })),
        FsOpStartCommand: vi.fn(async () => ({ op_id: "op1" })),
        FsOpResolveCommand: vi.fn(async () => ({})),
        FsOpCancelCommand: vi.fn(async () => ({})),
        ListNamedAgentsCommand: vi.fn(async () => [
            { instance_name: "korp", definition_name: "korp", definition_id: "def-korp", working_directory: "/agents/korp" },
            { instance_name: "loap", definition_name: "loap", definition_id: "def-loap", working_directory: "/agents/loap" },
        ]),
        RemotesListCommand: vi.fn(async () => [
            { name: "user@box", kind: "ssh", sources: ["recent"], settings: {} },
            { name: "wsl://Ubuntu", kind: "wsl", sources: ["wsl"], settings: {} },
        ]),
        GetAgentContentCommand: vi.fn(async (_c: unknown, req: { agent_id: string }) =>
            req.agent_id === "def-korp" ? { agent_id: req.agent_id, content_type: "ui:color", content: "#22c55e", updated_at: 0 } : null
        ),
    };
    return { state, rpc };
});

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
vi.mock("@/app/store/block-atom-cache", async (orig) => ({
    ...(await orig<typeof import("@/app/store/block-atom-cache")>()),
    getSettingsKeyAtom: () => () => undefined,
}));

import { setPlatform } from "@/util/platformutil";
import { setClipboard } from "./files-ops";
import { FilesModel } from "./files-model";
import { FilesView } from "./files-view";

const entry = (name: string, over: Partial<FsEntry> = {}): FsEntry => ({ name, is_dir: false, is_symlink: false, hidden: false, readonly: false, size: 10, mtime: 0, ...over });
const HOME = "/home/a";

function mount(meta: Record<string, unknown> = {}) {
    const [m, setM] = createSignal<Record<string, unknown>>(meta);
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
        visibility: () => "active",
    };
    const model = new FilesModel(ctx);
    const r = render(() => <FilesView model={model} ctx={ctx} />);
    const view = () => r.container.querySelector(".files-view") as HTMLElement;
    return { ...r, model, view, list: () => r.container.querySelector(".files-list") as HTMLElement };
}

/** One element per line, so a snapshot diff points at the element that changed. */
const markup = (...els: (Element | null)[]): string =>
    els.map((e) => (e ? e.outerHTML : "(absent)").replace(/></g, ">\n<")).join("\n----\n") + "\n";

/** What sits below the list (progress bars, the status line) and outside the pane (the conflict dialog). */
const footer = (v: ReturnType<typeof mount>): string => {
    const kids = [...v.view().children];
    const at = kids.findIndex((k) => k.classList.contains("files-body"));
    const outside = [...document.body.children].filter((el) => !el.contains(v.view()));
    return markup(...kids.slice(at + 1), ...outside);
};

const settled = async (v: ReturnType<typeof mount>) => {
    await waitFor(() => expect(v.view().querySelectorAll(".files-rows .files-name").length).toBeGreaterThan(0));
    await waitFor(() => expect(v.view().textContent).toContain("korp"));
    await waitFor(() => expect(v.view().textContent).toContain("user@box"));
};
const opEvent = (data: Record<string, unknown>) =>
    h.state.byType.get("files:op")?.forEach((fn) => fn({ data: { op_id: "op1", kind: "copy", done_items: 0, total_items: 1, done_bytes: 0, total_bytes: 0, ...data } }));

beforeEach(() => {
    setPlatform("linux");
    h.state.dirs.clear();
    h.state.byType = new Map();
    setClipboard(null);
    h.state.dirs.set(HOME, [entry("b.txt"), entry("src", { is_dir: true, size: undefined }), entry("a.md")]);
    h.state.dirs.set("~", [entry("notes.md")]);
    for (const fn of Object.values(h.rpc)) fn.mockClear();
    (globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class {
        observe() {}
        disconnect() {}
    };
});
afterEach(() => {
    cleanup();
    setPlatform("darwin");
});

const snap = (name: string) => `./__snapshots__/files-view.chrome.${name}.html`;

describe("the Files pane's chrome", () => {
    it("a local folder: toolbar, every Places section, column header, status", async () => {
        const v = mount();
        await settled(v);
        await expect(markup(v.view().querySelector(".files-toolbar"), v.view().querySelector(".files-places"), v.view().querySelector(".files-header"))).toMatchFileSnapshot(snap("local"));
        expect(footer(v)).toMatchInlineSnapshot(`
          "<div class="files-status" role="status">
          <span>3 items</span>
          </div>
          "
        `);
    });

    it("on an SSH host: the host crumb and the active Remote place", async () => {
        h.state.dirs.set("/home/u", [entry("notes.md")]);
        const v = mount({ connection: "user@box", "files:path": "/home/u" });
        await settled(v);
        await expect(markup(v.view().querySelector(".files-toolbar"), v.view().querySelector(".files-places"))).toMatchFileSnapshot(snap("remote"));
    });

    it("the path editor, the filter, and the toggles pressed", async () => {
        const v = mount();
        await settled(v);
        fireEvent.dblClick(v.view().querySelector(".files-breadcrumb")!);
        await expect(markup(v.view().querySelector(".files-toolbar"))).toMatchFileSnapshot(snap("path-editing"));
        fireEvent.keyDown(v.view().querySelector(".files-path-input")!, { key: "Escape" });
        fireEvent.click(v.view().querySelector('[aria-label^="Filter ("]')!);
        const input = await waitFor(() => v.view().querySelector(".files-filter-input") as HTMLInputElement);
        fireEvent.input(input, { target: { value: "md" } });
        fireEvent.click(v.view().querySelector('[aria-label="Show as thumbnails"]')!);
        fireEvent.click(v.view().querySelector('[aria-label="Show hidden files"]')!);
        fireEvent.click(v.view().querySelector('[aria-label="Hide Places"]')!);
        fireEvent.click(v.view().querySelector('[aria-label^="Show preview"]')!);
        await waitFor(() => expect(v.view().querySelector(".files-places")).toBeNull());
        await expect(markup(v.view().querySelector(".files-toolbar"), v.view().querySelector(".files-places"))).toMatchFileSnapshot(snap("toggled"));
        expect(v.view().querySelector(".files-status")?.textContent).toBe("1 of 3 items match");
    });

    it("a running copy, a conflict, and a finished one with its status", async () => {
        const v = mount();
        await settled(v);
        setClipboard({ kind: "copy", paths: ["/tmp/big.iso"] });
        fireEvent.keyDown(v.list(), { key: "v", ctrlKey: true });
        await waitFor(() => expect(h.rpc.FsOpStartCommand).toHaveBeenCalled());
        opEvent({ state: "running", done_bytes: 50, total_bytes: 200 });
        await waitFor(() => expect(v.view().querySelector(".files-op-text")?.textContent).toBe("Copying big.iso · 25%"));
        await expect(footer(v)).toMatchFileSnapshot(snap("op-running"));
        opEvent({ state: "conflict", conflict: { source: "/tmp/big.iso", dest: `${HOME}/big.iso`, source_is_dir: false, dest_is_dir: false, source_size: 2048, dest_size: 10 } });
        await waitFor(() => expect(screen.getByText("“big.iso” already exists here")).toBeTruthy());
        await expect(footer(v)).toMatchFileSnapshot(snap("op-conflict"));
        opEvent({ state: "done", done_items: 1 });
        await waitFor(() => expect(v.view().querySelector(".files-op")).toBeNull());
        await expect(footer(v)).toMatchFileSnapshot(snap("op-done"));
    });

    it("a status with Undo, and the git summary", async () => {
        h.rpc.FsGitStatusCommand.mockResolvedValue({ in_repo: true, branch: "main", ahead: 1, behind: 2, changes: 3, entries: [] });
        const v = mount();
        await settled(v);
        await waitFor(() => expect(v.view().querySelector(".files-git-summary")).not.toBeNull());
        await expect(footer(v)).toMatchFileSnapshot(snap("git"));
        fireEvent.click([...v.view().querySelectorAll(".files-rows .files-row")].find((r) => r.textContent?.includes("b.txt"))!);
        fireEvent.keyDown(v.list(), { key: "Delete" });
        await waitFor(() => expect(v.view().querySelector(".files-status")?.textContent).toContain("Moved b.txt to Trash"));
        await expect(footer(v)).toMatchFileSnapshot(snap("undo"));
    });
});

describe("the right-click menus", () => {
    const rowOf = (v: ReturnType<typeof mount>, name: string) =>
        [...v.view().querySelectorAll(".files-rows .files-row")].find((r) => r.textContent?.includes(name)) as HTMLElement;
    const menu = () => markup(document.querySelector(".ctx-menu"));

    it("a file: open, editor, default app, reveal, mention, cut, copy, rename, trash, delete", async () => {
        const v = mount();
        await settled(v);
        fireEvent.contextMenu(rowOf(v, "b.txt"));
        await expect(menu()).toMatchFileSnapshot(snap("menu-file"));
    });

    it("a folder: new tab and terminal instead of editor and default app", async () => {
        const v = mount();
        await settled(v);
        fireEvent.contextMenu(rowOf(v, "src"));
        await expect(menu()).toMatchFileSnapshot(snap("menu-folder"));
    });

    it("several rows: counts in the labels, and Rename off", async () => {
        const v = mount();
        await settled(v);
        fireEvent.click(rowOf(v, "b.txt"));
        fireEvent.click(rowOf(v, "a.md"), { ctrlKey: true });
        fireEvent.contextMenu(rowOf(v, "a.md"));
        await expect(menu()).toMatchFileSnapshot(snap("menu-many"));
    });

    it("on an SSH host: nothing that hands its paths to this computer", async () => {
        h.state.dirs.set("/home/u", [entry("notes.md"), entry("proj", { is_dir: true, size: undefined })]);
        const v = mount({ connection: "user@box", "files:path": "/home/u" });
        await settled(v);
        fireEvent.contextMenu(rowOf(v, "notes.md"));
        await expect(menu()).toMatchFileSnapshot(snap("menu-remote-file"));
    });

    it("the empty space: new entries, Paste with what the clipboard holds, and the folder's own actions", async () => {
        const v = mount();
        await settled(v);
        fireEvent.contextMenu(v.list());
        await expect(menu()).toMatchFileSnapshot(snap("menu-folder-blank"));
        cleanup();
        setClipboard({ kind: "cut", paths: ["/tmp/a", "/tmp/b"] });
        const w = mount();
        await settled(w);
        fireEvent.contextMenu(w.list());
        expect(document.querySelector(".ctx-menu")?.textContent).toContain("Paste (move 2)");
    });
});
