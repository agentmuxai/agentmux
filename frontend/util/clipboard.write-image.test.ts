// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// writeImage: the host puts the PNG on the clipboard; the browser's own call is the way when the host can't.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ api: {} as Record<string, unknown> }));
vi.mock("@/util/app-api", () => ({ getApi: () => h.api }));

import { writeImage } from "./clipboard";

const png = new Blob([new Uint8Array([1, 2, 3])], { type: "image/png" });

describe("writeImage", () => {
    const write = vi.fn(async () => {});
    beforeEach(() => {
        write.mockClear();
        h.api = {};
        vi.stubGlobal("ClipboardItem", class { constructor(readonly items: Record<string, Blob>) {} });
        Object.defineProperty(navigator, "clipboard", { configurable: true, value: { write } });
    });
    afterEach(() => vi.unstubAllGlobals());

    it("hands the host the PNG as base64, and stops there", async () => {
        const host = vi.fn(async (_png: string) => {});
        h.api = { writeClipboardImage: host };
        await writeImage(png);
        expect(host).toHaveBeenCalledWith("AQID");
        expect(write).not.toHaveBeenCalled();
    });

    it("falls back to the browser's clipboard when the host can't", async () => {
        h.api = { writeClipboardImage: vi.fn(async () => Promise.reject(new Error("can't copy an image without xclip (install it)"))) };
        await writeImage(png);
        expect(write).toHaveBeenCalledTimes(1);
    });

    it("a host with no image command uses the browser's", async () => {
        await writeImage(png);
        expect(write).toHaveBeenCalledTimes(1);
    });

    it("when both fail, the host's reason is the one thrown", async () => {
        h.api = { writeClipboardImage: vi.fn(async () => Promise.reject(new Error("can't copy an image without xclip (install it)"))) };
        write.mockRejectedValueOnce(new Error("NotAllowedError"));
        await expect(writeImage(png)).rejects.toThrow("can't copy an image without xclip (install it)");
    });

    it("with neither, says the window can't", async () => {
        Object.defineProperty(navigator, "clipboard", { configurable: true, value: undefined });
        await expect(writeImage(png)).rejects.toThrow("this window can't copy an image");
    });
});
