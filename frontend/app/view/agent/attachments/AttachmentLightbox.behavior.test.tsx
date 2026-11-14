// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AttachmentLightbox (the full-window preview of one attachment), pinned as a whole ahead of splitting it into a state
 * object and view parts: each scenario mounts the real lightbox over stand-ins for the pane modal, the kit's buttons,
 * the file-fetching hooks and the info call, drives the arrow and Escape keys and the buttons, and logs the lightbox
 * as an indented tree and, in order, every call it makes (the info it asks for, the ids the hooks are given, the
 * callbacks it fires). The logs are snapshots recorded on the unsplit component; the split must leave them byte for
 * byte. (AttachmentLightbox.test.tsx asserts the processing and missing notes against the real modal layer.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createEffect, createSignal, onMount, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const [urlValue, setUrl] = createSignal<string | null | undefined>(undefined);
const [textValue, setText] = createSignal<string | null | undefined>(undefined);
const h = vi.hoisted(() => ({ info: null as any }));

vi.mock("@/app/element/modal", () => ({
    Modal: (p: any) => {
        log(`Modal open=${p.open} scope=${p.scope} size=${p.size} panelClass=${p.panelClass}`);
        onMount(() => log(`initialFocus -> ${p.initialFocus?.()?.className}`));
        return (
            <div data-stand="Modal" data-aria-label={p.ariaLabel}>
                {p.children}
            </div>
        );
    },
}));
vi.mock("@/app/element/ui", () => ({
    CloseButton: (p: { label?: string; tooltip?: boolean; onClick?: () => void }) => (
        <span
            data-stand="CloseButton"
            data-label={p.label ?? "Close"}
            data-tooltip={String(p.tooltip)}
            onClick={() => (log(`CloseButton ${p.label ?? "Close"} click`), p.onClick?.())}
        />
    ),
    Button: (p: { tone?: string; onClick?: () => void; children?: JSX.Element }) => (
        <span data-stand="Button" data-tone={p.tone} onClick={() => (log(`Button ${p.children} click`), p.onClick?.())}>
            {p.children}
        </span>
    ),
    IconButton: (p: { icon: string; label: string; tooltip?: boolean; title?: string; onClick?: () => void }) => (
        <span
            data-stand="IconButton"
            data-icon={p.icon}
            data-label={p.label}
            data-tooltip={String(p.tooltip)}
            data-title={p.title}
            onClick={() => (log(`IconButton ${p.label} click`), p.onClick?.())}
        />
    ),
}));
vi.mock("@/app/element/tip", () => ({ tip: (text?: string) => ({ "data-tip": text }) }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AttachmentsInfoCommand: (_client: unknown, req: unknown) => (
            log(`AttachmentsInfoCommand ${JSON.stringify(req)}`),
            h.info()
        ),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("./attachment-url", () => ({
    useAttachmentUrl: (id: () => string | undefined, variant: string) => {
        createEffect(() => log(`useAttachmentUrl ${variant} id=${id()}`));
        return urlValue;
    },
    useAttachmentText: (id: () => string | undefined, variant: string) => {
        createEffect(() => log(`useAttachmentText ${variant} id=${id()}`));
        return textValue;
    },
}));

import { AttachmentLightbox, type LightboxItem } from "./AttachmentLightbox";

const SNAP = (name: string) => `./__snapshots__/AttachmentLightbox.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|src|alt|tabindex)$/;

/** The lightbox as an indented tree: tags, classes, the attributes that carry state, and text. */
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
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const flush = async () => {
    for (let i = 0; i < 12; i++) await Promise.resolve();
};
const click = (label: string) => {
    const b = [...host.querySelectorAll<HTMLElement>("[data-stand='IconButton'],[data-stand='CloseButton'],[data-stand='Button']")].find(
        (x) => x.dataset.label === label || x.textContent?.trim() === label
    );
    fireEvent.click(b!);
};
const dialog = () => host.querySelector<HTMLElement>(".agent-attachment-lightbox__dialog")!;
const key = (k: string) => {
    const e = new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true });
    e.preventDefault = () => log(`preventDefault ${k}`);
    e.stopPropagation = () => log(`stopPropagation ${k}`);
    dialog().dispatchEvent(e);
};

const IMG_INFO = {
    kind: "image",
    bytes: 2_500_000,
    width: 4000,
    height: 3000,
    send_width: 2000,
    send_height: 1500,
    send_bytes: 400_000,
    first_frame_only: false,
};
const item = (over: Partial<LightboxItem> = {}): LightboxItem => ({
    key: "k1",
    number: 1,
    name: "shot.png",
    id: "a1",
    info: IMG_INFO as any,
    ...over,
});

const onClose = vi.fn(() => log("onClose"));
const onRemove = vi.fn((k: string) => log(`onRemove ${k}`));

function mount(items: LightboxItem[], opts: { index?: number; remove?: boolean } = {}) {
    const [index, setIndex] = createSignal(opts.index ?? 0);
    const view = render(() => (
        <AttachmentLightbox
            items={items}
            index={index()}
            onIndex={(i) => (log(`onIndex ${i}`), setIndex(i))}
            onClose={onClose}
            onRemove={opts.remove ? onRemove : undefined}
        />
    ));
    host = view.container;
    return view;
}

beforeEach(() => {
    T.length = 0;
    setUrl(undefined);
    setText(undefined);
    h.info = async () => ({ items: [IMG_INFO] });
    onClose.mockClear();
    onRemove.mockClear();
});
afterEach(() => cleanup());

describe("AttachmentLightbox, pinned", () => {
    it("shows an image from its send copy with both sizes (and says when it is a first frame), loading and missing until the URL arrives", async () => {
        mount([item()]);
        look("loading");
        setUrl("blob:send/1");
        look("the image");
        cleanup();
        T.push("-- gone from the store");
        setUrl(null);
        mount([item()]);
        look("missing");
        cleanup();
        T.push("-- a first frame, and a title hover");
        setUrl("blob:send/2");
        mount([item({ info: { ...IMG_INFO, first_frame_only: true } as any })]);
        look("first frame only");
        return snapshot("image");
    });

    it("shows a text file's first lines, a note when it was cut, and loading or missing until the text arrives", async () => {
        const textInfo = { kind: "text", bytes: 12 };
        mount([item({ name: "notes.txt", info: textInfo as any })]);
        look("loading");
        setText("hello\nworld");
        look("whole file, no note");
        cleanup();
        T.push("-- cut");
        setText("hello");
        mount([item({ name: "notes.txt", info: { kind: "text", bytes: 5000 } as any })]);
        look("the first lines, with the note");
        cleanup();
        T.push("-- gone");
        setText(null);
        mount([item({ name: "notes.txt", info: textInfo as any })]);
        look("missing");
        return snapshot("text");
    });

    it("shows a file with no preview as its type icon and the facts the agent is given: size, pages, the text version, a note and the macros warning", async () => {
        mount([
            item({
                name: "report.pdf",
                info: {
                    kind: "pdf",
                    bytes: 1_200_000,
                    page_count: 12,
                    text_bytes: 4000,
                    text_note: "Scanned pages were skipped.",
                } as any,
            }),
        ]);
        look("a pdf");
        cleanup();
        T.push("-- one page, macros");
        mount([
            item({ name: "budget.xlsm", info: { kind: "excel", bytes: 2048, page_count: 1, macros: true } as any }),
        ]);
        look("a workbook with macros");
        cleanup();
        T.push("-- no details yet");
        mount([item({ name: "archive.zip", info: undefined, id: "z1" })]);
        look("an archive, details not yet fetched");
        return snapshot("details");
    });

    it("says why there is nothing to show: a failure, still processing, no id, and fetches the details of an item that only has an id", async () => {
        mount([item({ error: "Couldn't read the file." })]);
        look("an error");
        cleanup();
        T.push("-- pending");
        mount([item({ pending: true, id: undefined, info: undefined })]);
        look("processing");
        cleanup();
        T.push("-- no id");
        mount([item({ id: undefined, info: undefined, name: "scan.pdf" })]);
        look("gone");
        cleanup();
        T.push("-- the transcript only knows the id");
        setUrl("blob:send/9");
        mount([item({ info: undefined })]);
        look("before the details arrive");
        await flush();
        look("after the details arrive");
        cleanup();
        T.push("-- an item with its info asks for nothing");
        mount([item()]);
        await flush();
        return snapshot("states");
    });

    it("pages with the arrow keys and the buttons (wrapping), closes on Escape or the close button, removes when it can, and ignores other keys", async () => {
        const items = [
            item(),
            item({ key: "k2", number: 2, name: "two.png", id: "a2" }),
            item({ key: "k3", number: 3, name: "three.png", id: "a3" }),
        ];
        mount(items, { remove: true });
        look("first of three, with remove");
        key("ArrowRight");
        key("ArrowRight");
        key("ArrowRight");
        look("wrapped back to the first");
        key("ArrowLeft");
        look("wrapped to the last");
        click("Previous image");
        click("Next image");
        key("a");
        click("Remove");
        key("Escape");
        click("Close preview");
        cleanup();
        T.push("-- one item: no paging, no remove");
        mount([item()]);
        look("a single item");
        key("ArrowRight");
        cleanup();
        T.push("-- the index past the end shows the last");
        mount(items, { index: 7 });
        look("index clamped");
        return snapshot("paging");
    });

    it("closes itself when the last item goes", async () => {
        const [items, setItems] = createSignal<LightboxItem[]>([item()]);
        const view = render(() => (
            <AttachmentLightbox items={items()} index={0} onIndex={() => {}} onClose={onClose} />
        ));
        host = view.container;
        setItems([]);
        look("no items");
        return snapshot("empty");
    });
});
