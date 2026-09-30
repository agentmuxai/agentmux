// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Inline video and audio in the agent's own messages
 * (SPEC_AGENT_PANE_RICH_OUTPUT_2026_09_27.md §5, P4): the extension picks the
 * element; before play only a small `Range` read happens (a poster for video,
 * the size for audio); the whole file loads on play, under a 200 MB cap;
 * nothing autoplays on its own; codec failures say why; remote media is a link;
 * and a srv that ignores `Range` degrades to a name-and-size placeholder.
 */

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ fetch: vi.fn(), createBlock: vi.fn(async () => "b") }));
vi.mock("@/util/fetchutil", () => ({ fetch: (...a: unknown[]) => h.fetch(...a) }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://srv" }));
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));
vi.mock("@/app/store/block-layout-actions", () => ({ createBlock: h.createBlock }));

import { Markdown } from "./markdown";

const MB = 1024 * 1024;
let urls = 0;

/** srv with Range support: a 206 for a range, else the whole file. */
function serve(total: number, type = "video/webm") {
    h.fetch.mockImplementation(async (_url: string, init: RequestInit) => {
        const range = (init.headers as Record<string, string>)?.Range;
        const m = range && /bytes=(\d+)-(\d+)/.exec(range);
        if (m) {
            const start = Number(m[1]);
            const end = Math.min(Number(m[2]), total - 1);
            return {
                ok: true,
                status: 206,
                headers: new Headers({ "Content-Range": `bytes ${start}-${end}/${total}`, "Content-Length": String(end - start + 1) }),
                blob: async () => new Blob([new Uint8Array(end - start + 1)], { type }),
                body: { cancel: async () => {} },
            };
        }
        return {
            ok: true,
            status: 200,
            headers: new Headers({ "Content-Length": String(total) }),
            blob: async () => new Blob([new Uint8Array(8)], { type }),
            body: { cancel: async () => {} },
        };
    });
}

const rangeOf = (call: number) => ((h.fetch.mock.calls[call][1] as RequestInit).headers as Record<string, string>).Range;

beforeEach(() => {
    h.fetch.mockReset();
    h.createBlock.mockClear();
    urls = 0;
    vi.stubGlobal("URL", Object.assign(URL, { createObjectURL: () => `blob:av/${++urls}`, revokeObjectURL: () => {} }));
    // jsdom has no media playback.
    vi.spyOn(HTMLMediaElement.prototype, "play").mockImplementation(async () => {});
});
afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
});

const mount = (text: string) => render(() => <Markdown text={text} scrollable={false} media={{ baseDir: "C:/work" }} />).container;

describe("the extension decides the element", () => {
    it.each([
        ["clip.mp4", "video"],
        ["clip.webm", "video"],
        ["clip.mov", "video"],
        ["take.wav", "audio"],
    ])("%s → %s", async (file, el) => {
        serve(4 * MB);
        const c = mount(`![repro](${file})`);
        await waitFor(() => expect(c.querySelector(`.am-media-${el}`)).not.toBeNull());
        expect(c.querySelector("img")).toBeNull();
    });
});

describe("video", () => {
    it("reads only the first 2 MB for a poster, and nothing plays on its own", async () => {
        serve(50 * MB);
        const c = mount("![repro](clip.webm)");
        await waitFor(() => expect(c.querySelector("video.am-media-poster")).not.toBeNull());
        expect(h.fetch).toHaveBeenCalledTimes(1);
        expect(rangeOf(0)).toBe(`bytes=0-${2 * MB - 1}`);
        const poster = c.querySelector("video.am-media-poster") as HTMLVideoElement;
        expect(poster.autoplay).toBe(false);
        expect(poster.hasAttribute("controls")).toBe(false);
        expect(c.textContent).toContain("clip.webm");
        expect(c.textContent).toContain("50 MB");
    });

    it("fetches the whole file on play and plays it muted, with controls", async () => {
        serve(50 * MB);
        const c = mount("![repro](clip.webm)");
        await waitFor(() => expect(c.querySelector(".am-media-play")).not.toBeNull());
        fireEvent.click(c.querySelector(".am-media-play")!);
        await waitFor(() => expect(c.querySelector("video[controls]")).not.toBeNull());
        expect(rangeOf(1)).toBeUndefined();
        const v = c.querySelector("video[controls]") as HTMLVideoElement;
        expect(v.muted).toBe(true);
        expect(v.getAttribute("src")).toMatch(/^blob:/);
    });

    it("says why a codec can't play, instead of just 'failed'", async () => {
        serve(4 * MB, "video/mp4");
        const c = mount("![repro](clip.mp4)");
        await waitFor(() => expect(c.querySelector(".am-media-play")).not.toBeNull());
        fireEvent.click(c.querySelector(".am-media-play")!);
        await waitFor(() => expect(c.querySelector("video[controls]")).not.toBeNull());
        const v = c.querySelector("video[controls]") as HTMLVideoElement;
        Object.defineProperty(v, "error", { value: { code: 4, message: "" } });
        fireEvent.error(v);
        await waitFor(() => expect(c.textContent).toContain("MEDIA_ERR_SRC_NOT_SUPPORTED"));
    });

    it("frees the played file's blob as soon as playback fails (Codex P2 on #4073)", async () => {
        const revoked: string[] = [];
        vi.stubGlobal("URL", Object.assign(URL, { createObjectURL: () => `blob:av/${++urls}`, revokeObjectURL: (u: string) => revoked.push(u) }));
        serve(4 * MB, "video/mp4");
        const c = mount("![repro](clip.mp4)");
        await waitFor(() => expect(c.querySelector(".am-media-play")).not.toBeNull());
        fireEvent.click(c.querySelector(".am-media-play")!);
        await waitFor(() => expect(c.querySelector("video[controls]")).not.toBeNull());
        const v = c.querySelector("video[controls]") as HTMLVideoElement;
        const src = v.getAttribute("src")!;
        Object.defineProperty(v, "error", { value: { code: 4, message: "" } });
        fireEvent.error(v);
        await waitFor(() => expect(c.textContent).toContain("can't play"));
        expect(revoked).toContain(src);
    });

    it("shows a card that opens the Media pane over the 200 MB cap", async () => {
        serve(300 * MB);
        const c = mount("![repro](big.mp4)");
        await waitFor(() => expect(c.querySelector(".am-media-card")).not.toBeNull());
        expect(c.querySelector(".am-media-play")).toBeNull();
        expect(c.querySelector(".am-media-card")!.textContent).toContain("300 MB");
        fireEvent.click(c.querySelector(".am-media-card")!);
        expect(h.createBlock).toHaveBeenCalledWith({ meta: { view: "media", "media:path": "C:/work/big.mp4" } });
    });
});

describe("a srv without Range support", () => {
    // Before the stream-local-file Range change ships, srv answers a ranged
    // request with the whole file (200): no poster, but the name, size and
    // play button still work, and the body is cancelled, not downloaded.
    it("shows the name and size without a poster, and still plays", async () => {
        const cancel = vi.fn(async () => {});
        const blob = vi.fn(async () => new Blob([new Uint8Array(8)], { type: "video/webm" }));
        h.fetch.mockImplementation(async () => ({
            ok: true,
            status: 200,
            headers: new Headers({ "Content-Length": String(40 * MB) }),
            blob,
            body: { cancel },
        }));
        const c = mount("![repro](clip.webm)");
        await waitFor(() => expect(c.querySelector(".am-media-play")).not.toBeNull());
        expect(cancel).toHaveBeenCalledTimes(1);
        expect(blob).not.toHaveBeenCalled();
        expect(c.querySelector("video.am-media-poster")).toBeNull();
        expect(c.textContent).toContain("clip.webm");
        expect(c.textContent).toContain("40 MB");
        fireEvent.click(c.querySelector(".am-media-play")!);
        await waitFor(() => expect(c.querySelector("video[controls]")).not.toBeNull());
    });
});

describe("audio", () => {
    it("reads a single byte for the size before play, then the whole file", async () => {
        serve(3 * MB, "audio/wav");
        const c = mount("![voice note](take.wav)");
        await waitFor(() => expect(c.querySelector(".am-media-play")).not.toBeNull());
        expect(h.fetch).toHaveBeenCalledTimes(1);
        expect(rangeOf(0)).toBe("bytes=0-0");
        expect(c.textContent).toContain("3 MB");
        fireEvent.click(c.querySelector(".am-media-play")!);
        await waitFor(() => expect(c.querySelector("audio[controls]")).not.toBeNull());
    });
});

describe("remote media", () => {
    it("is a plain link, never loaded", () => {
        const c = mount("![demo](https://example.com/demo.mp4)");
        const a = c.querySelector("a");
        expect(a?.getAttribute("href")).toBe("https://example.com/demo.mp4");
        expect(c.querySelector("video, audio, img, .am-media-chip")).toBeNull();
        expect(h.fetch).not.toHaveBeenCalled();
    });
});

describe("Operator Config tells agents about video and audio", () => {
    const manifest = JSON.parse(
        readFileSync(join(__dirname, "../../../crates/srv/operator-config-seed.json"), "utf8"),
    ) as { version: number; entries: { id: string; instructions: string }[] };
    const entry = manifest.entries.find((e) => e.id === "operator-config-rich-output")!;

    it("describes them, in a newer manifest generation than P3's", () => {
        expect(manifest.version).toBeGreaterThanOrEqual(5);
        // WebM in the example: H.264 MP4 needs the proprietary-codec CEF build.
        expect(entry.instructions).toMatch(/!\[[^\]]*\]\([^)]*\.(mp4|webm)\)/);
        expect(entry.instructions).toContain(".wav");
    });
});
