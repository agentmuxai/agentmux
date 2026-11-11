// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane's preview panel (`FilesPreview` in files-preview.tsx), pinned as a whole ahead of splitting it into
 * a state file and view parts: each scenario mounts the real panel over the real kit, with stand-ins for the media
 * reads (whose answers the scenario controls), the code highlighter and the Markdown renderer, on a fake clock and
 * with the object URLs and the locale's date format stubbed, feeds it the selected entry through a signal, and logs
 * the panel as an indented tree and, in order, every call made (each read with its request, an aborted read, each
 * object URL made or revoked, each button's callback). The logs are snapshots recorded on the unsplit panel; the
 * split must leave them byte for byte.
 * (files-view.test.tsx asserts the preview inside the pane; this pins the panel's markup and calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { FsEntry } from "@/types/rpc/FsEntry";

const h = vi.hoisted(() => {
    const T: string[] = [];
    const log = (line: string): void => {
        T.push(line);
    };
    const state = {
        blob: {} as Record<string, () => Promise<Blob>>,
        range: {} as Record<string, () => Promise<{ blob: Blob | null; total: number }>>,
        urls: 0,
    };
    return { T, log, state };
});
const { T, log } = h;

vi.mock("@/app/element/local-media", async (orig) => ({
    ...(await orig<typeof import("@/app/element/local-media")>()),
    fetchMediaBlob: (path: string, opts: any) => {
        log(`fetchMediaBlob ${path} ${JSON.stringify({ maxBytes: opts.maxBytes, blockId: opts.blockId, type: opts.type })}`);
        opts.signal?.addEventListener("abort", () => log(`abort ${path}`));
        return (h.state.blob[path] ?? (() => Promise.resolve(new Blob(["png"]))))();
    },
    fetchMediaRange: (path: string, start: number, end: number, opts: any) => {
        log(`fetchMediaRange ${path} ${start}-${end} ${JSON.stringify({ blockId: opts.blockId })}`);
        opts.signal?.addEventListener("abort", () => log(`abort ${path}`));
        return (h.state.range[path] ?? (() => Promise.resolve({ blob: new Blob(["hello"]), total: 5 })))();
    },
}));
vi.mock("@/app/element/markdown", () => ({
    Markdown: (props: any) => (
        <div data-markdown="" data-scrollable={String(props.scrollable)}>
            {props.text}
        </div>
    ),
}));
vi.mock("../agent/components/HighlightedCode", () => ({
    HighlightedCode: (props: any) => (
        <pre data-highlighted="" data-lang={props.lang} class={props.class}>
            {props.code}
        </pre>
    ),
}));

import { FilesPreview } from "./files-preview";

const SNAP = (name: string) => `./__snapshots__/files-preview.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|id|src|alt)$/;

/** A subtree as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").replace(/\s+/g, " ").trim();
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

const entry = (name: string, extra: Partial<FsEntry> = {}): FsEntry => ({
    name,
    is_dir: false,
    is_symlink: false,
    size: 2048,
    mtime: 1700000000000,
    hidden: false,
    readonly: false,
    ...extra,
});

let host: HTMLElement;
let setEntry: (e: FsEntry | null) => void;
let unmount: () => void;
const look = (label: string) => {
    log(`-- ${label}`);
    const panel = host.querySelector(".files-preview");
    log(panel ? dump(panel).join("\n") : "no panel");
};
const flush = async (ms = 0) => {
    vi.advanceTimersByTime(ms);
    for (let i = 0; i < 12; i++) await Promise.resolve();
};
/** Selects `e`, waits out the pause and the read, and logs the panel. */
const select = async (label: string, e: FsEntry | null) => {
    log(`-- select ${label}`);
    setEntry(e);
    await flush(150);
    await flush();
    look(label);
};
const press = async (label: string, selector = ".files-preview") => {
    const found = [...host.querySelectorAll<HTMLElement>(`${selector} button`)].find((b) => b.textContent?.trim() === label)!;
    log(`-- press ${label}`);
    fireEvent.click(found);
    await flush();
};

beforeEach(() => {
    T.length = 0;
    h.state.blob = {};
    h.state.range = {};
    h.state.urls = 0;
    vi.useFakeTimers();
    vi.spyOn(Date.prototype, "toLocaleString").mockImplementation(function (this: Date) {
        return `T${this.getTime()}`;
    });
    (URL as unknown as { createObjectURL: unknown }).createObjectURL = () => {
        const url = `blob:img-${++h.state.urls}`;
        log(`createObjectURL ${url}`);
        return url;
    };
    (URL as unknown as { revokeObjectURL: unknown }).revokeObjectURL = (url: string) => log(`revokeObjectURL ${url}`);
    const [e, set] = createSignal<FsEntry | null>(null);
    setEntry = set;
    const r = render(() => (
        <FilesPreview
            entry={e()}
            pathOf={(name) => `/home/me/${name}`}
            blockId="blk-1"
            onOpen={(x) => log(`onOpen ${x.name}`)}
            onOpenWithOs={(x) => log(`onOpenWithOs ${x.name}`)}
        />
    ));
    host = r.container;
    unmount = r.unmount;
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
});

describe("FilesPreview, pinned as a whole", () => {
    it("shows each kind of preview", async () => {
        look("nothing selected");
        await select("folder", entry("src", { is_dir: true, size: undefined }));
        await select("image", entry("shot.png"));
        await select("svg image", entry("logo.svg"));
        await select("code", entry("main.rs"));
        h.state.range["/home/me/script"] = () => Promise.resolve({ blob: new Blob(["#!/usr/bin/env python\nprint(1)\n"]), total: 31 });
        await select("code, language from the first line", entry("script"));
        h.state.range["/home/me/notes.md"] = () => Promise.resolve({ blob: new Blob(["# Title\n\nbody"]), total: 13 });
        await select("markdown", entry("notes.md"));
        await select("empty file", entry("empty.txt", { size: 0 }));
        await select("empty markdown file", entry("empty.md", { size: 0 }));
        await select("video", entry("clip.mp4"));
        await select("audio", entry("song.mp3"));
        h.state.range["/home/me/tool.dat"] = () => Promise.resolve({ blob: new Blob([new Uint8Array([77, 90, 0, 1])]), total: 4 });
        await select("binary", entry("tool.dat"));
        await select("error entry", entry("locked.txt", { error: "Permission denied" }));
        await select("image too large", entry("huge.png", { size: 30 * 1024 * 1024 }));
        h.state.range["/home/me/ignored.bin"] = () => Promise.resolve({ blob: null, total: 0 });
        await select("range ignored", entry("ignored.bin"));
        await select("no size or time", entry("bare.txt", { size: undefined, mtime: undefined }));
        await select("nothing selected again", null);
        await snapshot("kinds");
    });

    it("says when only the start of a text file is shown", async () => {
        h.state.range["/home/me/big.log"] = () => Promise.resolve({ blob: new Blob(["line one\nline two"]), total: 5_000_000 });
        await select("truncated text", entry("big.log", { size: 5_000_000 }));
        h.state.range["/home/me/exact.log"] = () => Promise.resolve({ blob: new Blob(["abc"]), total: 3 });
        await select("whole file in range", entry("exact.log", { size: 3 }));
        h.state.range["/home/me/big.md"] = () => Promise.resolve({ blob: new Blob(["# Big"]), total: 900_000 });
        await select("truncated markdown", entry("big.md", { size: 900_000 }));
        await snapshot("truncated");
    });

    it("runs each button's callback", async () => {
        await select("folder", entry("src", { is_dir: true, size: undefined }));
        await press("Open folder");
        await select("image", entry("shot.png"));
        log("-- double-click the image");
        fireEvent.dblClick(host.querySelector(".files-preview-media img")!);
        await flush();
        h.state.range["/home/me/big.log"] = () => Promise.resolve({ blob: new Blob(["x"]), total: 5_000_000 });
        await select("truncated text", entry("big.log", { size: 5_000_000 }));
        await press("Open the whole file");
        await select("video", entry("clip.mp4"));
        await press("Play in a Media pane");
        h.state.range["/home/me/tool.dat"] = () => Promise.resolve({ blob: new Blob([new Uint8Array([0])]), total: 1 });
        await select("binary", entry("tool.dat"));
        await press("Open with default app");
        await select("error entry", entry("locked.txt", { error: "Permission denied" }));
        await press("Open with default app");
        await snapshot("actions");
    });

    it("reads only after the selection settles, and drops what the selection left behind", async () => {
        log("-- select a.txt");
        setEntry(entry("a.txt"));
        await flush(149);
        look("149 ms in: nothing read yet");
        log("-- select b.txt");
        setEntry(entry("b.txt"));
        await flush(149);
        look("149 ms after the second selection: still nothing read");
        await flush(1);
        look("150 ms: loading");
        await flush();
        look("b.txt read");

        // A read still in flight when the selection moves on is abandoned.
        let finishA!: (b: Blob) => void;
        h.state.blob["/home/me/a.png"] = () => new Promise<Blob>((res) => (finishA = res));
        log("-- select a.png (read stays open)");
        setEntry(entry("a.png"));
        await flush(150);
        look("a.png loading");
        log("-- select b.txt before a.png answers");
        setEntry(entry("b.txt", { size: 4096 }));
        await flush(150);
        await flush();
        look("b.txt again");
        log("-- a.png answers late");
        finishA(new Blob(["png"]));
        await flush();
        look("late image dropped");

        // The image's object URL is released when the selection moves on, and when the panel goes away.
        await select("image", entry("c.png"));
        await select("text after image", entry("d.txt"));
        await select("image again", entry("e.png"));
        await select("nothing selected", null);
        await select("image once more", entry("f.png"));
        log("-- unmount");
        unmount();
        await flush();
        await snapshot("timing");
    });

    it("reads again when the file changes on disk, and shows a failed read as the reason", async () => {
        await select("text", entry("main.rs"));
        log("-- same entry again (nothing changes)");
        setEntry(entry("main.rs"));
        await flush(150);
        await flush();
        look("not read again");
        await select("modified time changed", entry("main.rs", { mtime: 1700000100000 }));
        await select("size changed", entry("main.rs", { mtime: 1700000100000, size: 4096 }));
        h.state.range["/home/me/gone.txt"] = () => Promise.reject(new Error("404 Not Found"));
        await select("read fails with an Error", entry("gone.txt"));
        h.state.range["/home/me/odd.txt"] = () => Promise.reject("plain failure");
        await select("read fails with a string", entry("odd.txt"));
        h.state.blob["/home/me/bad.png"] = () => Promise.reject(new Error("image refused"));
        await select("image read fails", entry("bad.png"));
        await snapshot("reread");
    });
});
