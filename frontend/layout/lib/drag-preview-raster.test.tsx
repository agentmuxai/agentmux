// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const lib = vi.hoisted(() => ({
    fontCalls: 0,
    fontNodes: [] as unknown[],
    pngOptions: [] as unknown[],
    fontFails: false,
}));
vi.mock("html-to-image", () => ({
    getFontEmbedCSS: async (node: unknown) => {
        lib.fontCalls++;
        lib.fontNodes.push(node);
        if (lib.fontFails) throw new Error("no fonts");
        return "@font-face{}";
    },
    toPng: async (_node: unknown, options: unknown) => {
        lib.pngOptions.push(options);
        return "data:image/png;base64,x";
    },
}));

import { rasterisePane, resetDragPreviewRaster, warmDragPreview } from "./drag-preview-raster";

const node = {} as HTMLElement;
const fontFaces = (n: number) => Object.defineProperty(document, "fonts", { value: { size: n }, configurable: true });

beforeEach(() => {
    resetDragPreviewRaster();
    lib.fontCalls = 0;
    lib.fontNodes = [];
    lib.pngOptions = [];
    lib.fontFails = false;
    fontFaces(3);
});

describe("rasterisePane", () => {
    it("embeds the page's fonts once and hands the same CSS to every picture after", async () => {
        for (let i = 0; i < 3; i++) await rasterisePane(node, { pixelRatio: 1 });
        expect(lib.fontCalls).toBe(1);
        expect(lib.pngOptions).toEqual(Array(3).fill({ pixelRatio: 1, fontEmbedCSS: "@font-face{}" }));
    });

    it("embeds them again when the document has a new font face", async () => {
        await rasterisePane(node, {});
        fontFaces(4);
        await rasterisePane(node, {});
        expect(lib.fontCalls).toBe(2);
    });

    it("draws the picture as it always did when the fonts could not be embedded, and tries again next time", async () => {
        lib.fontFails = true;
        await rasterisePane(node, { pixelRatio: 1 });
        expect(lib.pngOptions).toEqual([{ pixelRatio: 1 }]);
        lib.fontFails = false;
        await rasterisePane(node, { pixelRatio: 1 });
        expect(lib.fontCalls).toBe(2);
        expect(lib.pngOptions[1]).toEqual({ pixelRatio: 1, fontEmbedCSS: "@font-face{}" });
    });
});

describe("warmDragPreview", () => {
    it("embeds the fonts from the pane's preview element, never the page, and only once", async () => {
        const idle: (() => void)[] = [];
        vi.stubGlobal("requestIdleCallback", (fn: () => void) => void idle.push(fn));
        const preview = { id: "preview" } as unknown as HTMLElement;
        warmDragPreview(() => preview, {});
        warmDragPreview(() => preview, {});
        expect(idle).toHaveLength(1);
        expect(lib.fontCalls).toBe(0);
        idle[0]();
        await vi.waitFor(() => expect(lib.fontCalls).toBe(1));
        expect(lib.fontNodes).toEqual([preview]);
        await rasterisePane(preview, {});
        expect(lib.fontCalls).toBe(1);
        vi.unstubAllGlobals();
    });

    it("waits for a later tile when no preview element is mounted yet", async () => {
        const idle: (() => void)[] = [];
        vi.stubGlobal("requestIdleCallback", (fn: () => void) => void idle.push(fn));
        warmDragPreview(() => undefined, {});
        idle[0]();
        expect(lib.fontCalls).toBe(0);
        warmDragPreview(() => ({}) as HTMLElement, {});
        expect(idle).toHaveLength(2);
        vi.unstubAllGlobals();
    });
});
