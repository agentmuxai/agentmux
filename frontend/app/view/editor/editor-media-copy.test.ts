// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What the Editor copies for the viewer in front: an image, a video frame, audio and a PDF each get their own items,
// and a text tab gets none (the pane's plain Copy stays).

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({ pushNotification: vi.fn() }));
vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { showContextMenu: vi.fn() } }));
vi.mock("@/util/clipboard", () => ({ writeImage: vi.fn(), writeText: vi.fn() }));
vi.mock("./viewers/viewer-tabs", () => ({
    isDroppedPath: (p: string) => p.startsWith("agentmux-dropped:/"),
    droppedFileOf: () => new File([new Uint8Array(4)], "d.png", { type: "image/png" }),
}));

import { editorCopyMenuItems, editorMediaSource } from "./editor-media-copy";

function model(over: { viewer: string; path: string; mime?: string }) {
    return {
        blockId: "b1",
        activeTabAtom: () => ({ id: "t1", filePath: over.path }),
        activeViewer: () => over.viewer,
        viewerTabs: { probe: () => (over.mime ? { mime: over.mime } : null) },
    } as never;
}

const labels = (m: never, selection = "") => editorCopyMenuItems(m, selection).map((i) => i.label);

describe("the Editor's Copy for media", () => {
    it.each([
        ["an image", { viewer: "image", path: "C:/a.png", mime: "image/png" }, ["Copy", "Copy path"]],
        ["a video, by its type", { viewer: "media", path: "C:/a.mp4", mime: "video/mp4" }, ["Copy frame", "Copy path"]],
        ["audio, by its type", { viewer: "media", path: "C:/a.wav", mime: "audio/wav" }, ["Copy path"]],
        ["audio, by its extension when the type is unknown", { viewer: "media", path: "C:/a.flac" }, ["Copy path"]],
        ["a PDF with no selection", { viewer: "pdf", path: "C:/a.pdf", mime: "application/pdf" }, ["Copy path"]],
    ])("%s offers %j", (_n, over, expected) => {
        expect(labels(model(over))).toEqual(expected);
    });

    it("a PDF's selected text keeps a Copy", () => {
        expect(labels(model({ viewer: "pdf", path: "C:/a.pdf" }), "text")).toEqual(["Copy", "Copy path"]);
    });

    it.each(["text", "hex", "large", "info"])("the %s viewer has none, so the pane's own Copy stays", (viewer) => {
        expect(editorMediaSource(model({ viewer, path: "C:/a.txt" }))).toBeNull();
        expect(labels(model({ viewer, path: "C:/a.txt" }))).toEqual([]);
    });

    it("a dropped file has no path to copy, and its bytes are the file's own", async () => {
        const source = editorMediaSource(model({ viewer: "image", path: "agentmux-dropped:/id/d.png" }))!;
        expect(source.path).toBeUndefined();
        expect((await source.bytes!()).size).toBe(4);
        expect(source.mime).toBe("image/png");
        expect(labels(model({ viewer: "image", path: "agentmux-dropped:/id/d.png" }))).toEqual(["Copy"]);
    });

    it("the video's frame comes from the video in this pane", () => {
        document.body.innerHTML = '<div data-blockid="b1"><video id="mine"></video></div><div data-blockid="b2"><video id="other"></video></div>';
        const source = editorMediaSource(model({ viewer: "media", path: "C:/a.mp4", mime: "video/mp4" }))!;
        expect((source.element!() as HTMLElement).id).toBe("mine");
    });
});
