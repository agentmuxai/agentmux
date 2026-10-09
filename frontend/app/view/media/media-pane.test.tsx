// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A Media pane's files as document tabs, and opening media into the Media
 * pane on screen. docs/specs/SPEC_DOCUMENT_TABS_2026_10_02.md §6.3, §5.7.
 */

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";

const hub = vi.hoisted(() => ({
    hooks: new Map<string, { drop: (d: { paths: string[]; files: File[] }) => void }>(),
    urls: 0,
    blocks: new Map<string, { meta: Record<string, unknown> }>(),
    leafs: [] as string[],
    focused: "" as string,
    metaWrites: [] as [string, Record<string, unknown>][],
}));

vi.mock("@/app/drag/file-drop", () => ({
    registerFileDropTarget: (id: string, hook: never) => {
        hub.hooks.set(id, hook);
        return () => hub.hooks.delete(id);
    },
}));
vi.mock("@/app/drag/file-drop-actions", () => ({ notifyDrop: { cantOpen: vi.fn() } }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { WatchMediaDirCommand: () => Promise.resolve(), UnwatchMediaDirCommand: () => Promise.resolve() },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: () => () => {} }));
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));
// The desktop host: native dialogs, so the empty pane offers to pick a file.
vi.mock("@/app/host/host-caps", () => ({ hostHas: () => true }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://x" }));
vi.mock("@/util/fetchutil", () => ({
    fetch: () => Promise.resolve({ ok: true, blob: () => Promise.resolve(new Blob(["png"])) }),
}));
vi.mock("@/app/store/mos", async (orig) => ({
    ...(await orig<typeof import("@/app/store/mos")>()),
    getObjectValue: (oref: string | null) => (oref ? hub.blocks.get(oref.replace(/^block:/, "")) : undefined),
}));
vi.mock("@/app/store/block-meta", () => ({
    setBlockMeta: async (id: string, meta: Record<string, unknown>) => void hub.metaWrites.push([id, meta]),
}));
vi.mock("@/layout/lib/layoutModelHooks", () => ({
    getLayoutModelForStaticTab: () => ({
        focusedNode: () => ({ data: { blockId: hub.focused } }),
        leafs: () => hub.leafs.map((blockId) => ({ data: { blockId } })),
    }),
}));

import { mediaPaneTab } from "./media";
import { openInMediaPaneOnScreen } from "./media-open";
import { MediaPaneModel } from "./media-pane";

let reqs = 0;
/** A `media:open` request, as openInMediaPaneOnScreen appends it. */
const req = (path: string) => ({ id: `r${++reqs}`, path });

beforeEach(() => {
    hub.hooks.clear();
    hub.urls = 0;
    hub.blocks.clear();
    hub.leafs = [];
    hub.focused = "";
    hub.metaWrites = [];
    URL.createObjectURL = () => `blob:test/${++hub.urls}`;
    URL.revokeObjectURL = () => {};
});
afterEach(() => cleanup());

/** `lagOpenClear`: the queue's clearing never reaches the block (a slow
 *  round trip), while other writes do. */
function mount(meta: Record<string, unknown> = {}, opts: { lagOpenClear?: boolean } = {}) {
    const [m, setM] = createSignal<Record<string, unknown>>(meta);
    const ctx: PaneTabHostContext = {
        blockId: "m1",
        meta: () => m() as MetaType,
        setMeta: async (patch) => {
            const next = { ...m() };
            for (const [k, v] of Object.entries(patch)) {
                if (opts.lagOpenClear && k === "media:open" && v === null) continue;
                if (v === null) delete next[k];
                else next[k] = v;
            }
            setM(next);
        },
        isFocused: () => true,
        visibility: () => "active",
    };
    const inst = mediaPaneTab.create!(ctx);
    const r = render(() => inst.component({ ctx } as never));
    const pills = () => [...r.container.querySelectorAll(".doc-tab-strip .pane-tab-label")].map((x) => x.textContent);
    const root = () => r.container.querySelector(".media-pane") as HTMLElement;
    return { ...r, inst, meta: m, setMeta: ctx.setMeta, pills, root };
}

describe("the Media pane's document tabs", () => {
    it("a pane from before tabs shows its file, with no strip for one tab", async () => {
        const v = mount({ "media:path": "C:/pics/a.png" });
        await waitFor(() => expect(v.container.querySelector("img")).not.toBeNull());
        expect(v.container.querySelector(".doc-tab-strip")).toBeNull();
        expect(v.inst.liveTitle!().text).toBe("a.png");
    });

    it("files sent here become tabs; the empty tab takes the first; the queue empties", async () => {
        const v = mount();
        expect(v.container.textContent).toContain("Click to load media");
        await v.setMeta({ "media:open": [req("C:/pics/a.png"), req("C:/clips/b.mp4")] });
        await waitFor(() => expect(v.pills()).toEqual(["a.png", "b.mp4"]));
        await waitFor(() => expect(v.meta()["media:open"]).toBeUndefined());
        expect(v.meta()["media:path"]).toBe("C:/clips/b.mp4");
        // Only the tab in front is mounted: one player, no hidden image.
        await waitFor(() => expect(v.container.querySelector("video")).not.toBeNull());
        expect(v.container.querySelector("img")).toBeNull();
    });

    it("switching tabs sticks while the queue's clearing is still on its way (live-test bug)", async () => {
        const v = mount({ "media:path": "C:/pics/a.png" }, { lagOpenClear: true });
        await v.setMeta({ "media:open": [req("C:/pics/b.png")] });
        await waitFor(() => expect(v.pills()).toEqual(["a.png", "b.png"]));
        fireEvent.keyDown(v.root(), { key: "PageUp", ctrlKey: true });
        await waitFor(() => expect(v.inst.liveTitle!().text).toBe("a.png"));
        await new Promise((r) => setTimeout(r, 20));
        expect(v.inst.liveTitle!().text).toBe("a.png");
        fireEvent.keyDown(v.root(), { key: "t", ctrlKey: true });
        await waitFor(() => expect(v.pills()).toEqual(["a.png", "Media", "b.png"]));
    });

    it("a second request for the same file is handled even while the first one's clearing is on its way (muxreview on #4235)", async () => {
        const v = mount({ "media:path": "C:/pics/a.png" }, { lagOpenClear: true });
        const first = req("C:/pics/b.png");
        await v.setMeta({ "media:open": [first] });
        await waitFor(() => expect(v.inst.liveTitle!().text).toBe("b.png"));
        fireEvent.keyDown(v.root(), { key: "PageUp", ctrlKey: true });
        await waitFor(() => expect(v.inst.liveTitle!().text).toBe("a.png"));
        // A second click on the same image, appended behind the first.
        await v.setMeta({ "media:open": [first, req("C:/pics/b.png")] });
        await waitFor(() => expect(v.inst.liveTitle!().text).toBe("b.png"));
        expect(v.pills()).toEqual(["a.png", "b.png"]);
    });

    it("one file, one tab: sent into an empty tab, the tab already showing it comes to the front (muxreview on #4235)", async () => {
        const v = mount({ "media:path": "C:/pics/a.png" });
        fireEvent.keyDown(v.root(), { key: "t", ctrlKey: true });
        await waitFor(() => expect(v.pills()).toEqual(["a.png", "Media"]));
        await v.setMeta({ "media:open": [req("C:/pics/a.png")] });
        await waitFor(() => expect(v.inst.liveTitle!().text).toBe("a.png"));
        expect(v.pills()).toEqual(["a.png", "Media"]);
    });

    it("a newer render landing on a file another tab shows leaves one tab on it", () => {
        const ctx = { blockId: "m9", meta: () => ({}) as MetaType, setMeta: async () => {}, isFocused: () => true, visibility: () => "active" } as PaneTabHostContext;
        const pane = new MediaPaneModel(ctx);
        pane.open("C:/out/1.png");
        pane.tabs.open({ path: "C:/out/2.png" });
        const shown = pane.tabs.activeId()!;
        // A picked or dropped file into an empty tab gives way, too.
        pane.tabs.newDocument();
        expect(pane.setTabPath(pane.tabs.activeId()!, "C:/out/1.png")).toBe(false);
        expect(pane.tabs.active()?.payload.path).toBe("C:/out/1.png");
        // The tab in front follows a newer render onto 1.png: 1.png's own tab goes.
        pane.tabs.activate(shown);
        expect(pane.setTabPath(shown, "C:/out/1.png")).toBe(true);
        expect(pane.tabs.tabs().map((t) => t.payload.path)).toEqual(["C:/out/1.png"]);
        expect(pane.tabs.activeId()).toBe(shown);
        pane.dispose();
    });

    it("dropped bytes open in a new tab, survive a tab switch, and aren't saved (muxreview on #4235)", async () => {
        const v = mount({ "media:path": "C:/pics/a.png" });
        await waitFor(() => expect(v.container.querySelector("img")).not.toBeNull());
        hub.hooks.get("m1")!.drop({ paths: [], files: [new File(["x"], "drop.png", { type: "image/png" })] });
        await waitFor(() => expect(v.pills()).toEqual(["a.png", "drop.png"]));
        expect(v.inst.liveTitle!().text).toBe("drop.png");
        await waitFor(() => expect(v.container.querySelector("img")).not.toBeNull());
        fireEvent.keyDown(v.root(), { key: "PageUp", ctrlKey: true });
        await waitFor(() => expect(v.inst.liveTitle!().text).toBe("a.png"));
        fireEvent.keyDown(v.root(), { key: "PageDown", ctrlKey: true });
        await waitFor(() => expect(v.inst.liveTitle!().text).toBe("drop.png"));
        await waitFor(() => expect(v.container.querySelector("img")).not.toBeNull());
        expect(v.container.textContent).not.toContain("Click to load media");
        // Saved, the bytes are nothing (no path): a restart restores a.png alone.
        await waitFor(() => expect((v.meta().doctabs as { tabs: unknown[] } | undefined)?.tabs).toHaveLength(2));
        const record = v.meta().doctabs;
        cleanup();
        const again = mount({ doctabs: record });
        expect(again.pills()).toEqual([]);
        expect(again.inst.liveTitle!().text).toBe("a.png");
    });

    it("a drop on a tab showing a file opens a new tab; on an empty tab, it shows there", async () => {
        const v = mount();
        hub.hooks.get("m1")!.drop({ paths: ["C:/pics/a.png"], files: [] });
        await waitFor(() => expect(v.inst.liveTitle!().text).toBe("a.png"));
        expect(v.pills()).toEqual([]);
        hub.hooks.get("m1")!.drop({ paths: ["C:/pics/b.png"], files: [] });
        await waitFor(() => expect(v.pills()).toEqual(["a.png", "b.png"]));
    });

    it("Ctrl+T adds an empty tab, Ctrl+W closes, and closing the last leaves an empty one", async () => {
        const v = mount({ "media:path": "C:/pics/a.png" });
        fireEvent.keyDown(v.root(), { key: "t", ctrlKey: true });
        await waitFor(() => expect(v.pills()).toEqual(["a.png", "Media"]));
        fireEvent.keyDown(v.root(), { key: "w", ctrlKey: true });
        await waitFor(() => expect(v.pills()).toEqual([]));
        fireEvent.keyDown(v.root(), { key: "w", ctrlKey: true });
        await waitFor(() => expect(v.inst.liveTitle!().text).toBe("Media"));
        expect(v.container.textContent).toContain("Click to load media");
        fireEvent.keyDown(v.root(), { key: "T", ctrlKey: true, shiftKey: true });
        await waitFor(() => expect(v.pills()).toEqual(["Media", "a.png"]));
    });

    it("keeps its files in the block and restores them", async () => {
        const v = mount();
        await v.setMeta({ "media:open": [req("C:/pics/a.png"), req("C:/pics/b.png")] });
        await waitFor(() => expect((v.meta().doctabs as { tabs: unknown[] } | undefined)?.tabs).toHaveLength(2));
        const record = v.meta().doctabs;
        cleanup();
        const again = mount({ doctabs: record });
        expect(again.pills()).toEqual(["a.png", "b.png"]);
        expect(again.inst.liveTitle!().text).toBe("b.png");
    });
});

describe("opening media into the Media pane on screen", () => {
    it("queues the file on the focused Media pane, else the first one, else says there is none", async () => {
        expect(await openInMediaPaneOnScreen("C:/x.png")).toBe(false);
        hub.blocks.set("t1", { meta: { view: "term" } });
        const queued = req("C:/a.png");
        hub.blocks.set("m1", { meta: { view: "media", "media:open": [queued] } });
        hub.blocks.set("m2", { meta: { view: "media" } });
        hub.leafs = ["t1", "m1", "m2"];
        hub.focused = "t1";
        expect(await openInMediaPaneOnScreen("C:/x.png")).toBe(true);
        expect(hub.metaWrites).toEqual([["m1", { "media:open": [queued, { id: expect.any(String), path: "C:/x.png" }] }]]);
        hub.focused = "m2";
        await openInMediaPaneOnScreen("C:/y.png");
        expect(hub.metaWrites[1]).toEqual(["m2", { "media:open": [{ id: expect.any(String), path: "C:/y.png" }] }]);
    });
});
