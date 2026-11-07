// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The inline video and audio player (LocalAV) and the lazy start it shares with the local image, pinned ahead of
 * splitting LocalAV into a state object and view parts: each scenario mounts the real `MarkdownImg` for a local media
 * path over stand-ins for the file fetches, the viewport observer, blob URLs and the Editor, drives the observer, the
 * play button and the media elements' errors, and logs the result as an indented tree and, in order, every call it
 * makes (the ranges and files it fetches, the URLs it makes and frees, the Editor it opens). The logs are snapshots
 * recorded on the unsplit component; the split must leave them byte for byte.
 * (markdown-media.test.tsx asserts what the person sees through `<Markdown>`; this pins the player's markup and calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({
    range: null as any,
    blob: null as any,
    io: null as any,
}));

vi.mock("@/app/element/local-media", async () => {
    const actual = await vi.importActual<typeof import("@/app/element/local-media")>("@/app/element/local-media");
    return {
        ...actual,
        fetchMediaRange: (path: string, start: number, end: number, opts: { signal?: AbortSignal }) => (
            log(`fetchMediaRange ${path} ${start}-${end} aborted=${opts?.signal?.aborted}`),
            h.range()
        ),
        fetchMediaBlob: (path: string, opts: { maxBytes?: number; type?: string; signal?: AbortSignal }) => (
            log(`fetchMediaBlob ${path} max=${opts?.maxBytes} type=${opts?.type} aborted=${opts?.signal?.aborted}`),
            h.blob()
        ),
        describeMediaError: (el: HTMLMediaElement | undefined) => `media error on ${el?.tagName?.toLowerCase()}`,
    };
});
vi.mock("@/app/motion/resize-contract", () => ({
    withHeightContinuity: (_el: HTMLElement, fn: () => void) => (log("withHeightContinuity"), fn()),
}));
vi.mock("@/app/store/block-layout-actions", () => ({
    createBlock: async (req: unknown) => (log(`createBlock ${JSON.stringify(req)}`), "new-block"),
}));
vi.mock("@/app/store/global", () => ({ openLink: (url: string) => log(`openLink ${url}`) }));
vi.mock("@/app/util/editor-open", () => ({
    openInEditorPaneOnScreen: async (path: string) => (log(`openInEditorPaneOnScreen ${path}`), false),
}));
vi.mock("@/app/element/tip", () => ({ tip: (text: string) => ({ "data-tip": text }) }));
vi.mock("@/element/ui", () => ({
    Button: (p: { class?: string; title?: string; onClick?: () => void; children?: JSX.Element }) => (
        <span
            data-stand="Button"
            class={p.class}
            data-title={p.title}
            onClick={() => (log(`Button ${p.class} click`), p.onClick?.())}
        >
            {p.children}
        </span>
    ),
}));

import { MarkdownImg } from "./markdown-media";

const SNAP = (name: string) => `./__snapshots__/markdown-media.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS =
    /^(class|role|aria-.*|data-.*|title|style|disabled|type|src|alt|preload|playsinline|controls|autoplay|muted)$/;

/** The player as an indented tree: tags, classes, the attributes that carry state, the media flags, and text. */
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
    if (el instanceof HTMLMediaElement) attrs.push(`muted.prop=${el.muted}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
let urls = 0;
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const flush = async () => {
    for (let i = 0; i < 12; i++) await Promise.resolve();
};
const defer = <T,>() => {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => ((resolve = res), (reject = rej)));
    return { promise, resolve, reject };
};
/** The viewport observer comes up with the element near the viewport. */
const approach = () => h.io.cb([{ isIntersecting: true }]);
const playButton = () => host.querySelector<HTMLElement>(".am-media-play")!;
const blobOf = (n: number) => new Blob([new Uint8Array(n)]);

class FakeObserver {
    cb: (entries: { isIntersecting: boolean }[]) => void;
    constructor(cb: (entries: { isIntersecting: boolean }[]) => void, opts: { rootMargin?: string }) {
        this.cb = cb;
        h.io = this;
        log(`IntersectionObserver rootMargin=${opts?.rootMargin}`);
    }
    observe(el: Element) {
        log(`observe ${el.tagName.toLowerCase()}.${el.className}`);
    }
    disconnect() {
        log("io.disconnect");
    }
}

function mount(src: string) {
    const view = render(() => <MarkdownImg props={{ src, alt: "clip" }} media={{ baseDir: "C:/work" }} />);
    host = view.container;
    return view;
}

beforeEach(() => {
    T.length = 0;
    urls = 0;
    h.io = null;
    h.range = async () => ({ blob: blobOf(8), total: 5_000_000 });
    h.blob = async () => blobOf(16);
    vi.stubGlobal("IntersectionObserver", FakeObserver);
    vi.stubGlobal(
        "URL",
        Object.assign(URL, {
            createObjectURL: () => (log(`createObjectURL blob:test/${urls + 1}`), `blob:test/${++urls}`),
            revokeObjectURL: (u: string) => log(`revokeObjectURL ${u}`),
        })
    );
});
afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
});

describe("LocalAV, pinned", () => {
    it("waits for the viewport, reads a poster range for a video, shows the size and a play button, then plays the whole file muted with controls", async () => {
        mount("clips/demo.mp4");
        look("waiting");
        approach();
        await flush();
        look("probed: a poster and a play button");
        const play = defer<Blob>();
        h.blob = () => play.promise;
        fireEvent.click(playButton());
        look("loading the whole file");
        fireEvent.click(playButton());
        play.resolve(blobOf(32));
        await flush();
        look("playing");
        cleanup();
        log("-- unmounted");
        return snapshot("video");
    });

    it("drops a poster the browser can't decode, and a video with no poster range shows only the stage", async () => {
        mount("clips/demo.mp4");
        approach();
        await flush();
        host.querySelector(".am-media-poster")!.dispatchEvent(new Event("error"));
        look("poster failed");
        cleanup();
        T.push("-- no poster bytes");
        h.range = async () => ({ blob: null, total: 5_000_000 });
        mount("clips/demo.mp4");
        approach();
        await flush();
        look("no poster, ready to play");
        return snapshot("poster");
    });

    it("reads one byte of an audio file for its size, shows a card with a play button, and plays it with controls", async () => {
        mount("sound/take.wav");
        look("waiting");
        approach();
        await flush();
        look("audio ready");
        fireEvent.click(playButton());
        await flush();
        look("audio playing");
        return snapshot("audio");
    });

    it("says why a file can't play: not found when the probe fails, a card when it is over the cap (at the probe or at play), and the player's own error (freeing the played file)", async () => {
        h.range = async () => {
            throw new Error("404");
        };
        mount("clips/gone.mp4");
        approach();
        await flush();
        look("missing at the probe");
        cleanup();
        T.push("-- over the cap at the probe");
        h.range = async () => ({ blob: blobOf(8), total: 300 * 1024 * 1024 });
        mount("clips/huge.mp4");
        approach();
        await flush();
        look("too large at the probe");
        fireEvent.click(host.querySelector(".am-media-card")!);
        await flush();
        cleanup();
        T.push("-- over the cap at play");
        h.range = async () => ({ blob: blobOf(8), total: 1000 });
        const { MediaTooLargeError } = await import("@/app/element/local-media");
        h.blob = async () => {
            throw new MediaTooLargeError(250 * 1024 * 1024);
        };
        mount("sound/long.mp3");
        approach();
        await flush();
        fireEvent.click(playButton());
        await flush();
        look("too large at play");
        cleanup();
        T.push("-- the player fails");
        h.blob = async () => blobOf(16);
        mount("clips/odd.mp4");
        approach();
        await flush();
        fireEvent.click(playButton());
        await flush();
        host.querySelector(".am-media-player")!.dispatchEvent(new Event("error"));
        look("playback error, the file freed");
        cleanup();
        T.push("-- the audio player fails");
        mount("sound/odd.ogg");
        approach();
        await flush();
        fireEvent.click(playButton());
        await flush();
        host.querySelector(".am-media-player")!.dispatchEvent(new Event("error"));
        look("audio playback error");
        return snapshot("failures");
    });

    it("probes at once where there is no viewport observer, ignores a probe that lands after the row is gone, and aborts and frees everything on unmount", async () => {
        vi.stubGlobal("IntersectionObserver", undefined);
        mount("clips/demo.mp4");
        await flush();
        look("no observer: probed at once");
        cleanup();
        T.push("-- unmounted before the probe lands");
        vi.stubGlobal("IntersectionObserver", FakeObserver);
        const late = defer<any>();
        h.range = () => late.promise;
        mount("clips/late.mp4");
        approach();
        cleanup();
        late.resolve({ blob: blobOf(8), total: 1000 });
        await flush();
        log("-- the late probe changed nothing");
        cleanup();
        T.push("-- unmounted while the file loads");
        h.range = async () => ({ blob: blobOf(8), total: 1000 });
        const play = defer<Blob>();
        h.blob = () => play.promise;
        mount("clips/slow.mp4");
        approach();
        await flush();
        fireEvent.click(playButton());
        cleanup();
        play.resolve(blobOf(32));
        await flush();
        log("-- the late file changed nothing");
        return snapshot("unmount");
    });

    it("starts a local image only when it nears the viewport, shows it from a blob URL, and says so when it is missing", async () => {
        h.blob = async () => blobOf(16);
        mount("shots/a.png");
        look("image waiting");
        approach();
        await flush();
        look("image ready");
        cleanup();
        T.push("-- missing");
        h.blob = async () => {
            throw new Error("404");
        };
        mount("shots/gone.png");
        approach();
        await flush();
        look("image missing");
        return snapshot("image");
    });
});
