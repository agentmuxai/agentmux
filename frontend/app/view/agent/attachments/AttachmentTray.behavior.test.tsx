// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AttachmentTray, pinned as a whole ahead of splitting it into a state object and view parts: each scenario
 * drives the tray over a draft (the tiles and the "+N" tile, expanding and collapsing, the summary and its limit
 * levels, processing and its Cancel, the announcements, removing with Undo and its timer, the tile keys, Clear,
 * the focus hand-offs and the lightbox) and logs the tray as a tree and, in order, every call it makes. The logs
 * are snapshots recorded on the unsplit component; the split must leave them byte for byte. The draft is a small
 * stand-in with the draft's own remove, restore and move; the tile, the lightbox and the thumbnail hook are
 * stand-ins that show the props they get.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DraftAttachment, RemovedAttachment } from "./attachment-draft";

const h = vi.hoisted(() => ({ log: [] as string[], resize: null as null | ((w: number) => void) }));

vi.mock("./attachment-url", () => ({ useAttachmentUrl: (id: () => string | undefined) => () => (id() ? `blob:${id()}` : null) }));
vi.mock("./AttachmentTile", () => ({
    AttachmentTile: (p: any) => (
        <li
            ref={p.ref}
            class="stub-tile"
            data-key={p.tile.key}
            data-number={p.tile.number}
            data-status={p.tile.status}
            data-kind={p.tile.kind}
            data-flash={p.flash ? "yes" : "no"}
            data-caption={p.caption ? "yes" : "no"}
            data-progress={p.tile.progress ?? ""}
            onKeyDown={p.onKeyDown}
        >
            <button class="agent-attachment-tile__main" onClick={() => p.onOpen()}>
                {p.tile.name}
            </button>
            <button class="stub-remove" aria-label={`Remove ${p.tile.name}`} onClick={() => p.onRemove()} />
        </li>
    ),
}));
vi.mock("./AttachmentLightbox", () => ({
    AttachmentLightbox: (p: any) => (
        <div class="stub-lightbox" data-index={p.index} data-items={p.items.map((i: any) => `${i.number}:${i.name}${i.pending ? "*" : ""}`).join(",")}>
            <button class="stub-lb-index" onClick={() => p.onIndex(p.items.length - 1)} />
            <button class="stub-lb-close" onClick={() => p.onClose()} />
            <button class="stub-lb-remove" onClick={() => p.onRemove(p.items[p.index].key)} />
        </div>
    ),
}));

import { AttachmentTray } from "./AttachmentTray";

const T = h.log;
const log = (line: string) => T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/AttachmentTray.behavior.${name}.txt`);

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|style)$/;
/** The tray as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    if (el.tagName === "svg" || el.tagName === "STYLE") return out;
    const attrs = [...el.attributes].filter((a) => ATTRS.test(a.name)).map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
const look = (label: string) => {
    log(`-- ${label}`);
    log(`   focus: ${describeFocus()}`);
    log(dump(host).join("\n"));
};
const describeFocus = () => {
    const a = document.activeElement as HTMLElement | null;
    if (!a || a === document.body) return "body";
    const li = a.closest("[data-key]") as HTMLElement | null;
    return li ? `tile ${li.dataset.key}` : a.id || a.className || a.tagName;
};

function makeDraft() {
    const [items, setItems] = createSignal<DraftAttachment[]>([]);
    const [flashKey, setFlashKey] = createSignal<string | null>(null);
    const [maxFiles, setMaxFiles] = createSignal(128);
    const [maxTotalBytes, setMaxTotalBytes] = createSignal(1024 * 1024 * 1024);
    const draft = {
        items, flashKey, maxFiles, maxTotalBytes, setItems, setFlashKey, setMaxFiles, setMaxTotalBytes,
        count: () => items().length,
        totalBytes: () => items().reduce((s, i) => s + i.bytes, 0),
        processingCount: () => items().filter((i) => i.status === "processing").length,
        progress: () => {
            const inFlight = items().filter((i) => i.status === "processing");
            const total = inFlight.reduce((s, i) => s + i.bytes, 0);
            const done = inFlight.reduce((s, i) => s + (i.stage === "processing" ? i.bytes : Math.min(i.doneBytes, i.bytes)), 0);
            return { done, total, count: inFlight.length };
        },
        remove(key: string): RemovedAttachment | null {
            const position = items().findIndex((i) => i.key === key);
            if (position < 0) return null;
            const item = items()[position];
            log(`draft remove ${key}`);
            setItems((prev) => prev.filter((i) => i.key !== key));
            return { item, position };
        },
        removeAll(): RemovedAttachment[] {
            log("draft removeAll");
            const list = items();
            setItems(() => []);
            return list.map((item, position) => ({ item, position }));
        },
        restore(removed: RemovedAttachment[]) {
            log(`draft restore ${removed.map((r) => r.item.key).join(",")}`);
            setItems((prev) => {
                const next = [...prev];
                for (const r of [...removed].sort((a, b) => a.position - b.position)) next.splice(Math.min(r.position, next.length), 0, r.item);
                return next;
            });
        },
        move(key: string, delta: number) {
            log(`draft move ${key} ${delta}`);
            setItems((prev) => {
                const from = prev.findIndex((i) => i.key === key);
                const to = from + delta;
                if (from < 0 || to < 0 || to >= prev.length) return prev;
                const next = [...prev];
                const [it] = next.splice(from, 1);
                next.splice(to, 0, it);
                return next;
            });
        },
        cancel: () => log("draft cancel"),
    };
    return draft;
}
type Draft = ReturnType<typeof makeDraft>;

const MB = 1024 * 1024;
const att = (n: number, over: Partial<DraftAttachment> = {}): DraftAttachment => ({
    key: `k${n}`,
    name: `pic${n}.png`,
    bytes: MB,
    status: "ready",
    stage: "uploading",
    doneBytes: MB,
    info: { id: `id${n}`, kind: "image", width: 10, height: 10 } as any,
    ...over,
});
const many = (n: number) => Array.from({ length: n }, (_, i) => att(i + 1));

let draft: Draft;
let focusLast: (() => void) | undefined;
const focusComposer = vi.fn(() => log("focusComposer"));
async function mount(items: DraftAttachment[] = []) {
    draft = makeDraft();
    draft.setItems(() => items);
    const view = render(() => <AttachmentTray draft={draft as any} focusComposer={focusComposer} registerFocusLast={(fn) => (focusLast = fn)} />);
    host = view.container;
    await flush();
    return view;
}
const flush = async () => {
    for (let i = 0; i < 3; i++) await vi.advanceTimersByTimeAsync(0);
};
const click = async (name: string | RegExp) => {
    fireEvent.click(screen.getByRole("button", { name }));
    await flush();
};
const tile = (key: string) => host.querySelector(`[data-key="${key}"]`) as HTMLElement;
const main = (key: string) => tile(key).querySelector(".agent-attachment-tile__main") as HTMLElement;
const key = async (k: string, init: KeyboardEventInit = {}, on = "k1") => {
    fireEvent.keyDown(tile(on), { key: k, ...init });
    await flush();
};

beforeEach(() => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    T.length = 0;
    focusComposer.mockClear();
    focusLast = undefined;
    h.resize = null;
    (globalThis as any).ResizeObserver = class {
        constructor(cb: (e: { contentRect: { width: number } }[]) => void) {
            h.resize = (w) => cb([{ contentRect: { width: w } }]);
        }
        observe() {}
        disconnect() {
            log("resize observer disconnected");
        }
    };
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("the tiles", () => {
    it("shows nothing but the live region with no attachments", async () => {
        await mount();
        look("empty");
        await snapshot("empty");
    });

    it("one row of tiles with the summary, in images and then in attachments once one is not an image", async () => {
        await mount(many(3));
        look("three images");
        draft.setItems((p) => [...p, att(4, { name: "notes.pdf", info: { id: "id4", kind: "pdf" } as any })]);
        await flush();
        look("a pdf added");
        await snapshot("row-and-summary");
    });

    it("warns, then errors, as the count and the size near and pass their limits", async () => {
        await mount(many(10));
        draft.setMaxFiles(12);
        await flush();
        look("10 of 12: warn on the count");
        draft.setMaxFiles(9);
        await flush();
        look("10 of 9: over on the count");
        draft.setMaxFiles(128);
        draft.setMaxTotalBytes(11 * MB);
        await flush();
        look("10 MB of 11 MB: warn on the size");
        draft.setMaxTotalBytes(5 * MB);
        await flush();
        look("10 MB of 5 MB: over on the size");
        await snapshot("limit-levels");
    });

    it("puts the rest behind a +N tile with a mosaic of the next four, expands, and collapses again", async () => {
        await mount(many(8));
        h.resize!(220);
        await flush();
        look("narrow: two tiles and +6");
        await click(/Show all 8 images/);
        look("expanded");
        await click("Show less");
        look("collapsed");
        await click(/Show all 8 images/);
        h.resize!(900);
        await flush();
        look("wide again: the overflow is gone, so it collapses");
        await snapshot("overflow-expand");
    });

    it("a tile the draft flashes is marked, and the mosaic of a mixed set shows icons for what is not a picture", async () => {
        const items = [...many(5), att(6, { name: "a.pdf", info: { id: "id6", kind: "pdf" } as any }), att(7, { status: "processing", info: undefined })];
        await mount(items);
        draft.setFlashKey("k2");
        h.resize!(150);
        await flush();
        look("flash on k2, mosaic of mixed");
        await snapshot("flash-and-mosaic");
    });
});

describe("processing and announcing", () => {
    it("shows the progress bar with Cancel, then says how many are ready and how many failed", async () => {
        await mount([att(1), att(2, { status: "processing", stage: "uploading", doneBytes: MB / 2, info: undefined }), att(3, { status: "processing", stage: "processing", info: undefined })]);
        look("processing two");
        await click("Cancel");
        draft.setItems((p) => p.map((i) => (i.key === "k2" ? { ...i, status: "ready", info: { id: "id2", kind: "image" } as any } : i.key === "k3" ? { ...i, status: "error", error: "bad file" } : i)));
        await flush();
        look("all done, one failed");
        await snapshot("processing");
    });

    it("says it is adding when more arrive, one line a batch", async () => {
        await mount(many(2));
        draft.setItems((p) => [...p, att(3, { status: "processing", info: undefined }), att(4, { status: "processing", info: undefined })]);
        await flush();
        look("adding two");
        draft.setItems((p) => p.map((i) => ({ ...i, status: "ready", info: i.info ?? ({ id: `id${i.key}`, kind: "image" } as any) })));
        await flush();
        look("ready");
        await snapshot("announce-adding");
    });
});

describe("removing and undoing", () => {
    it("a tile's remove shows an Undo line that goes after six seconds, and Undo brings it back", async () => {
        await mount(many(3));
        fireEvent.click(screen.getByRole("button", { name: "Remove pic2.png" }));
        await flush();
        look("removed pic2");
        await vi.advanceTimersByTimeAsync(5900);
        look("5.9 s: still there");
        await vi.advanceTimersByTimeAsync(200);
        look("6.1 s: gone");
        fireEvent.click(screen.getByRole("button", { name: "Remove pic1.png" }));
        await flush();
        await click("Undo");
        look("undone");
        await vi.advanceTimersByTimeAsync(7000);
        look("7 s later: no Undo line again");
        await snapshot("undo");
    });

    it("hovering or focusing the tray holds the Undo line until it is left", async () => {
        await mount(many(3));
        const tray = host.querySelector(".agent-attachment-tray") as HTMLElement;
        fireEvent.click(screen.getByRole("button", { name: "Remove pic3.png" }));
        await flush();
        fireEvent.mouseEnter(tray);
        await vi.advanceTimersByTimeAsync(20000);
        look("hovered for 20 s: still there");
        fireEvent.mouseLeave(tray);
        await vi.advanceTimersByTimeAsync(6100);
        look("left: gone");
        fireEvent.click(screen.getByRole("button", { name: "Remove pic2.png" }));
        await flush();
        fireEvent.focusIn(tray);
        await vi.advanceTimersByTimeAsync(20000);
        look("focused for 20 s: still there");
        fireEvent.focusOut(tray);
        await vi.advanceTimersByTimeAsync(6100);
        look("blurred: gone");
        await snapshot("undo-paused");
    });

    it("Clear removes everything with an Undo for all of it, and hands focus to the composer", async () => {
        await mount(many(3));
        await click("Clear");
        look("cleared");
        await click("Undo");
        look("undone");
        draft.setItems(() => [att(1)]);
        await flush();
        look("one attachment: no Clear");
        await snapshot("clear");
    });
});

describe("the keyboard", () => {
    it("Delete and Backspace remove the tile and put focus on its neighbour, or on the composer after the last", async () => {
        await mount(many(3));
        main("k1").focus();
        await key("Delete");
        look("deleted k1: focus on k2");
        await key("Backspace", {}, "k2");
        look("backspaced k2: focus on k3");
        await key("Delete", {}, "k3");
        look("deleted the last: composer");
        await snapshot("keys-remove");
    });

    it("the arrows move focus along the row, Alt+arrow moves the tile, Escape returns to the composer", async () => {
        await mount(many(3));
        main("k1").focus();
        await key("ArrowRight", {}, "k1");
        look("right from k1");
        await key("ArrowRight", {}, "k2");
        await key("ArrowRight", {}, "k3");
        look("right from the last: composer");
        await key("ArrowLeft", {}, "k2");
        look("left from k2");
        await key("ArrowLeft", {}, "k1");
        look("left from the first: the first");
        await key("ArrowRight", { altKey: true }, "k1");
        look("alt+right on k1: moved");
        await key("ArrowLeft", { altKey: true }, "k1");
        await key("Escape", {}, "k2");
        look("escape");
        await snapshot("keys-move");
    });

    it("the composer can move focus onto the last shown tile", async () => {
        await mount(many(3));
        focusLast!();
        look("focus last");
        await mount([]);
        focusLast!();
        look("nothing to focus");
        await snapshot("focus-last");
    });
});

describe("the lightbox", () => {
    it("opens on a tile, follows its index, removes from inside and adjusts, and closes when the last goes", async () => {
        await mount([att(1), att(2), att(3, { status: "processing", info: undefined })]);
        fireEvent.click(main("k2"));
        await flush();
        look("opened on the second");
        fireEvent.click(host.querySelector(".stub-lb-index") as HTMLElement);
        await flush();
        look("index moved to the last");
        fireEvent.click(host.querySelector(".stub-lb-remove") as HTMLElement);
        await flush();
        look("the last removed from inside");
        fireEvent.click(host.querySelector(".stub-lb-close") as HTMLElement);
        await flush();
        look("closed");
        fireEvent.click(main("k1"));
        await flush();
        fireEvent.click(host.querySelector(".stub-lb-remove") as HTMLElement);
        await flush();
        fireEvent.click(main("k2"));
        await flush();
        fireEvent.click(host.querySelector(".stub-lb-remove") as HTMLElement);
        await flush();
        look("all removed: the lightbox is gone");
        await snapshot("lightbox");
    });
});

describe("leaving", () => {
    it("stops watching the width and the Undo timer when the tray goes", async () => {
        const view = await mount(many(2));
        fireEvent.click(screen.getByRole("button", { name: "Remove pic1.png" }));
        await flush();
        view.unmount();
        log("-- unmounted");
        await vi.advanceTimersByTimeAsync(7000);
        log("-- seven seconds on");
        await snapshot("leave");
    });
});
