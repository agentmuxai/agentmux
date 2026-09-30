// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A file dropped onto a media pane that is already showing one.
 * SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.3 ("Later panes").
 */

import { cleanup, render } from "@solidjs/testing-library";
import { batch } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({ hooks: new Map<string, any>(), urls: 0, revoked: [] as string[] }));

vi.mock("@/app/drag/file-drop", () => ({
    registerFileDropTarget: (id: string, hook: unknown) => {
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
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://x" }));
vi.mock("@/util/fetchutil", () => ({
    fetch: () => Promise.resolve({ ok: true, blob: () => Promise.resolve(new Blob(["png"])) }),
}));

import { mediaPaneTab } from "./media";

const settle = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
    hub.hooks.clear();
    hub.urls = 0;
    hub.revoked = [];
    URL.createObjectURL = () => `blob:test/${++hub.urls}`;
    URL.revokeObjectURL = (u: string) => void hub.revoked.push(u);
});
afterEach(() => cleanup());

function mount(meta: Record<string, unknown>) {
    const ctx = { blockId: "m1", meta: () => meta, setMeta: vi.fn().mockResolvedValue(undefined) } as any;
    const { container } = render(() => mediaPaneTab.create(ctx).component({ ctx }));
    return { container, ctx };
}

describe("media pane: a pathless drop", () => {
    it("stays on screen when the pane was already showing a file", async () => {
        const { container } = mount({ "media:path": "C:/pics/old.png" });
        await settle();
        expect(container.querySelector("img")?.getAttribute("src")).toBe("blob:test/1");

        const file = new File(["x"], "dropped.png", { type: "image/png" });
        // Batched, the source-change effect runs after showFile returns: the
        // order a deferred effect gets in the app.
        batch(() => void hub.hooks.get("m1").drop({ paths: [], files: [file] }));
        await settle();
        await settle();

        expect(container.querySelector("img")?.getAttribute("src")).toBe("blob:test/2");
        expect(hub.revoked).toContain("blob:test/1");
        expect(hub.revoked).not.toContain("blob:test/2");
    });

    it("forgets the saved path, so a reload doesn't bring the old file back", async () => {
        const { ctx } = mount({ "media:path": "C:/pics/old.png" });
        await settle();
        await hub.hooks.get("m1").drop({ paths: [], files: [new File(["x"], "d.webm")] });
        expect(ctx.setMeta).toHaveBeenCalledWith({ "media:path": "" });
    });
});

// Codex P2 on #4064: an inline SVG in an agent message opens here on click.
describe("media pane: SVG", () => {
    it("shows an .svg file as an image", async () => {
        const { container } = mount({ "media:path": "C:/pics/diagram.svg" });
        await settle();
        expect(container.querySelector("img")?.getAttribute("src")).toBe("blob:test/1");
    });
});
