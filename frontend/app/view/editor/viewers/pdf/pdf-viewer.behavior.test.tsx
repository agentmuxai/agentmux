// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PdfViewer, pinned as a whole before it was split into a state object, hook groups and view parts: each scenario
 * drives the viewer against a stand-in for pdf.js (the document, the page viewer, the event bus, the range
 * transport) and for the file reads, and logs, in order, the rendered viewer as a tree and every call the viewer
 * made outward (pdf.js, the file reads, the pane's zoom, the zoom registrations). The logs are snapshots recorded
 * on the unsplit component; the split must leave them byte for byte.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
    const T: string[] = [];
    const log = (line: string) => void T.push(line);
    type Listener = (e: any) => void;
    const state = {
        T,
        log,
        docPages: 5,
        manual: false,
        tasks: [] as Array<{ id: number; resolve: (v?: unknown) => void; reject: (e: unknown) => void; onPassword?: any }>,
        bus: null as null | { emit: (name: string, payload?: unknown) => void },
        viewer: null as any,
        range: null as any,
        signals: [] as AbortSignal[],
        blobImpl: (async () => ({ arrayBuffer: async () => new ArrayBuffer(8) })) as (path: string) => Promise<unknown>,
        rangeImpl: (async (_p: string, b: number, e: number) => ({
            blob: { arrayBuffer: async () => new ArrayBuffer(e - b + 1) },
            total: 100 * 1024 * 1024,
        })) as (path: string, b: number, e: number) => Promise<unknown>,
        loaded: 0,
        taskCount: 0,
    };
    class EventBus {
        listeners: Record<string, Listener[]> = {};
        constructor() {
            state.bus = this;
        }
        on(name: string, fn: Listener) {
            (this.listeners[name] ??= []).push(fn);
        }
        emit(name: string, payload?: unknown) {
            (this.listeners[name] ?? []).forEach((fn) => fn(payload));
        }
        dispatch(name: string, payload: Record<string, unknown>) {
            log(`bus.dispatch ${name} ${JSON.stringify(payload)}`);
        }
    }
    class PDFLinkService {
        constructor(_o: unknown) {}
        setViewer(_v: unknown) {
            log("linkService.setViewer");
        }
        setDocument(d: { numPages: number } | null, base: unknown) {
            log(`linkService.setDocument pages=${d?.numPages} base=${base}`);
        }
    }
    class PDFFindController {
        constructor(_o: unknown) {}
    }
    class PDFViewer {
        container: HTMLElement;
        viewer: HTMLElement;
        linkService: unknown;
        eventBus: EventBus;
        pagesCount = 0;
        private page = 1;
        currentScale = 1;
        constructor(o: any) {
            state.viewer = this;
            this.container = o.container;
            this.viewer = o.viewer;
            this.linkService = o.linkService;
            this.eventBus = o.eventBus;
            state.signals.push(o.abortSignal);
            log(
                `new PDFViewer container=${o.container.className} viewer=${o.viewer.className} textLayerMode=${o.textLayerMode} annotationMode=${o.annotationMode} maxCanvasPixels=${o.maxCanvasPixels} signal=${o.abortSignal instanceof AbortSignal} aborted=${o.abortSignal.aborted}`
            );
        }
        get currentPageNumber() {
            return this.page;
        }
        set currentPageNumber(n: number) {
            log(`viewer.currentPageNumber = ${n}`);
            this.page = n;
            this.eventBus.emit("pagechanging", { pageNumber: n });
        }
        set currentScaleValue(v: string) {
            log(`viewer.currentScaleValue = ${v}`);
            this.currentScale = v === "page-width" ? 1.5 : 0.75;
            this.eventBus.emit("scalechanging", { scale: this.currentScale });
        }
        updateScale(o: { scaleFactor: number; drawingDelay?: number }) {
            log(`viewer.updateScale factor=${o.scaleFactor.toFixed(4)} delay=${o.drawingDelay}`);
            this.currentScale *= o.scaleFactor;
            this.eventBus.emit("scalechanging", { scale: this.currentScale });
        }
        setDocument(d: { numPages: number } | null) {
            log(`viewer.setDocument ${d ? "pages=" + d.numPages : "null"}`);
            this.pagesCount = d?.numPages ?? 0;
            if (!d) this.page = 1;
        }
    }
    class PDFDataRangeTransport {
        requestDataRange: (b: number, e: number) => void = () => {};
        constructor(length: number, initial: Uint8Array) {
            state.range = this;
            log(`new PDFDataRangeTransport length=${length} initial=${initial.length}`);
        }
        onDataRange(begin: number, bytes: Uint8Array) {
            log(`range.onDataRange begin=${begin} len=${bytes.length}`);
        }
    }
    const pdfjs = {
        AnnotationMode: { ENABLE: 2 },
        PasswordResponses: { NEEDED_PASSWORD: 1, INCORRECT_PASSWORD: 2 },
        PDFDataRangeTransport,
        getDocument(opts: Record<string, unknown>) {
            const id = ++state.taskCount;
            const keys = Object.keys(opts)
                .sort()
                .map((k) => {
                    const v = opts[k];
                    if (v instanceof Uint8Array) return `${k}=Uint8Array(${v.length})`;
                    if (v instanceof PDFDataRangeTransport) return `${k}=range`;
                    return `${k}=${JSON.stringify(v)}`;
                });
            log(`getDocument #${id} ${keys.join(" ")}`);
            let resolve!: (v: unknown) => void;
            let reject!: (e: unknown) => void;
            const entry: any = { id };
            const promise = state.manual
                ? new Promise((res, rej) => {
                      resolve = res;
                      reject = rej;
                  })
                : Promise.resolve({ numPages: state.docPages, id });
            entry.resolve = (v: unknown) => resolve(v ?? { numPages: state.docPages, id });
            entry.reject = (e: unknown) => reject(e);
            state.tasks.push(entry);
            const task: any = {
                promise,
                destroy: () => {
                    log(`task#${id}.destroy`);
                    return Promise.resolve();
                },
            };
            Object.defineProperty(task, "onPassword", {
                set(fn) {
                    entry.onPassword = fn;
                },
            });
            return task;
        },
    };
    const runtime = {
        SAFE_DOCUMENT_OPTIONS: { safe: true },
        async loadPdfjs() {
            state.loaded++;
            log("loadPdfjs");
            return { pdfjs, viewer: { EventBus, PDFLinkService, PDFFindController, PDFViewer } };
        },
    };
    return Object.assign(state, { runtime });
});

vi.mock("./pdfjs-runtime", () => h.runtime);
vi.mock("@/app/element/local-media", async (original) => ({
    ...(await original<typeof import("@/app/element/local-media")>()),
    fetchMediaBlob: async (path: string, o: { signal?: AbortSignal }) => {
        h.log(`fetchMediaBlob ${path} signal=${o.signal instanceof AbortSignal}`);
        return h.blobImpl(path);
    },
    fetchMediaRange: async (path: string, b: number, e: number, o: { signal?: AbortSignal }) => {
        h.log(`fetchMediaRange ${path} ${b}-${e} signal=${o.signal instanceof AbortSignal}`);
        return h.rangeImpl(path, b, e);
    },
}));
vi.mock("@/app/store/zoom", async (original) => ({
    ...(await original<typeof import("@/app/store/zoom")>()),
    setZoomStepRatio: (id: string, ratio: number) => {
        h.log(`setZoomStepRatio ${id} ${ratio}`);
        return () => h.log(`release zoomStepRatio ${id}`);
    },
    zoomBlockIn: (id: string, step: number) => h.log(`zoomBlockIn ${id} ${step}`),
    zoomBlockOut: (id: string, step: number) => h.log(`zoomBlockOut ${id} ${step}`),
    zoomBlockTo: (id: string, level: number) => h.log(`zoomBlockTo ${id} ${level}`),
}));
vi.mock("@/app/store/zoom-factor", async (original) => ({
    ...(await original<typeof import("@/app/store/zoom-factor")>()),
    setZoomMax: (id: string, max: number) => {
        h.log(`setZoomMax ${id} ${max}`);
        return () => h.log(`release zoomMax ${id}`);
    },
    clampZoom: (z: number, id: string) => {
        h.log(`clampZoom ${id} ${z.toFixed(4)}`);
        return z;
    },
}));
vi.mock("@/app/store/chrome-zoom", () => ({ chromeZoomAtom: () => 1 }));
vi.mock("../content-zoom", () => ({
    registerContentZoom: (id: string, handle: { scroller(): HTMLElement | undefined }) => {
        h.log(`registerContentZoom ${id} scroller=${handle.scroller()?.className}`);
        return () => h.log(`release contentZoom ${id}`);
    },
}));
vi.mock("@/app/keybindings", async (original) => ({
    ...(await original<typeof import("@/app/keybindings")>()),
    keyLabel: (k: string) => `[${k}]`,
}));

import { PdfViewer, WHOLE_FILE_MAX_BYTES, type PdfViewerProps } from "./pdf-viewer";

const SNAP = (name: string) => `./__snapshots__/PdfViewer.behavior.${name}.txt`;
const snapshot = (name: string) => expect(h.T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));
const log = h.log;

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|name|placeholder|for|href)$/;

/** The viewer as an indented tree: tags, classes, the attributes that carry state, form values and text. */
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
    const attrs = [...el.attributes]
        .filter((a) => ATTRS.test(a.name))
        .map((a) => `${a.name}=${JSON.stringify(a.value.replace(/(-cl)-\d+/g, "$1-N"))}`);
    if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) attrs.push(`value=${JSON.stringify(el.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

function show(label: string) {
    log(`--- ${label}`);
    const root = document.querySelector(".pdf-viewer");
    if (root) dump(root).forEach((l) => log(l));
    else log("(nothing rendered)");
}

const settle = async () => {
    for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
};

const q = <T extends Element = HTMLElement>(sel: string) => document.querySelector(sel) as T;
const byLabel = (label: string) => q(`[aria-label="${label}"]`);
const byText = (text: string) =>
    [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === text) as HTMLButtonElement;
const focusName = () => {
    const a = document.activeElement as HTMLElement | null;
    return a && a !== document.body ? `${a.tagName.toLowerCase()}.${a.className}` : "body";
};

interface Opts extends Partial<Pick<PdfViewerProps, "path" | "size" | "file" | "revision">> {
    zoom?: number;
}

function mount(opts: Opts = {}) {
    const [path, setPath] = createSignal(opts.path ?? "/docs/a.pdf");
    const [revision, setRevision] = createSignal(opts.revision ?? 1);
    const [size, setSize] = createSignal<number | undefined>(opts.size ?? 1000);
    const [file, setFile] = createSignal<File | undefined>(opts.file);
    const [zoom, setZoom] = createSignal(opts.zoom ?? 1);
    const view = render(() => (
        <PdfViewer
            path={path()}
            revision={revision()}
            size={size()}
            file={file()}
            blockId="blk-1"
            zoom={zoom}
            bar={{ viewer: "pdf", options: ["pdf"], onReopenWith: () => {} }}
            onOpenLink={(url) => log(`onOpenLink ${url}`)}
        />
    ));
    return { view, setPath, setRevision, setSize, setFile, setZoom };
}

/** Mount and let the document load and its pages initialise, as pdf.js does. */
async function ready(opts: Opts = {}) {
    const m = mount(opts);
    await settle();
    h.bus!.emit("pagesinit");
    return m;
}

beforeEach(() => {
    h.T.length = 0;
    h.docPages = 5;
    h.manual = false;
    h.tasks.length = 0;
    h.bus = null;
    h.viewer = null;
    h.range = null;
    h.signals.length = 0;
    h.taskCount = 0;
    h.blobImpl = async () => ({ arrayBuffer: async () => new ArrayBuffer(8) });
    h.rangeImpl = async (_p, b, e) => ({
        blob: { arrayBuffer: async () => new ArrayBuffer(e - b + 1) },
        total: 100 * 1024 * 1024,
    });
});
afterEach(() => cleanup());

describe("PdfViewer behavior", () => {
    it("opens a small file by path and walks the pages", async () => {
        const m = mount({ size: 123456 });
        show("mounted");
        await settle();
        show("loaded, pages not yet laid out");
        h.bus!.emit("pagesinit");
        show("pages laid out");
        fireEvent.click(byLabel("Next page"));
        fireEvent.click(byLabel("Next page"));
        show("on page 3");
        fireEvent.click(byLabel("Previous page"));
        const input = byLabel("Page") as HTMLInputElement;
        fireEvent.change(input, { target: { value: "5" } });
        show("typed page 5");
        fireEvent.change(input, { target: { value: "99" } });
        fireEvent.change(input, { target: { value: "abc" } });
        fireEvent.keyDown(input, { key: "Enter" });
        fireEvent.change(input, { target: { value: "-3" } });
        fireEvent.change(input, { target: { value: "2" } });
        fireEvent.keyDown(input, { key: "Enter" });
        fireEvent.keyDown(input, { key: "a" });
        fireEvent.keyDown(q(".pdf-viewer-container"), { key: "End" });
        fireEvent.keyDown(q(".pdf-viewer-container"), { key: "Home" });
        fireEvent.keyDown(q(".pdf-viewer-container"), { key: "End", ctrlKey: true });
        fireEvent.keyDown(input, { key: "Home" });
        show("keys");
        fireEvent.keyDown(q(".pdf-viewer-container"), { key: "g", metaKey: true });
        log(`focus after mod+g: ${focusName()}`);
        fireEvent.keyDown(q(".pdf-viewer-container"), { key: "F", ctrlKey: true });
        log(`focus after mod+f: ${focusName()}`);
        fireEvent.keyDown(q(".pdf-viewer-container"), { key: "G", ctrlKey: true });
        log(`focus after mod+G: ${focusName()}`);
        m.setSize(undefined);
        show("size unknown");
        await snapshot("walks-the-pages");
    });

    it("zooms with the buttons and follows the pane's zoom", async () => {
        const m = await ready();
        show("ready");
        fireEvent.click(byLabel("Zoom in ([mod+=])"));
        fireEvent.click(byLabel("Zoom out ([mod+-])"));
        fireEvent.click(byText("Width"));
        fireEvent.click(byText("Page"));
        show("after the buttons");
        m.setZoom(2);
        m.setZoom(0.5);
        show("after the pane's zoom changed");
        await snapshot("zooms");
    });

    it("ignores zoom buttons before a document is open", async () => {
        const m = mount();
        fireEvent.click(byLabel("Zoom in ([mod+=])"));
        fireEvent.click(byText("Width"));
        fireEvent.click(byText("Page"));
        fireEvent.click(byLabel("Next page"));
        fireEvent.keyDown(q(".pdf-viewer-container"), { key: "End" });
        fireEvent.input(byLabel("Find in the document"), { target: { value: "early" } });
        m.setZoom(3);
        show("still loading");
        await settle();
        await snapshot("ignores-early-zoom");
    });

    it("finds text", async () => {
        await ready();
        const find = byLabel("Find in the document") as HTMLInputElement;
        fireEvent.input(find, { target: { value: "cat" } });
        show("typed");
        h.bus!.emit("updatefindmatchescount", { matchesCount: { current: 1, total: 4 } });
        show("4 matches");
        fireEvent.keyDown(find, { key: "Enter" });
        fireEvent.keyDown(find, { key: "Enter", shiftKey: true });
        fireEvent.keyDown(find, { key: "x" });
        h.bus!.emit("updatefindcontrolstate", { matchesCount: { current: 2, total: 4 } });
        h.bus!.emit("updatefindcontrolstate", {});
        show("on match 2");
        fireEvent.input(find, { target: { value: "dog" } });
        h.bus!.emit("updatefindmatchescount", { matchesCount: { current: 0, total: 0 } });
        show("no matches");
        fireEvent.input(find, { target: { value: "" } });
        show("cleared");
        await snapshot("finds");
    });

    it("inverts the pages", async () => {
        await ready();
        fireEvent.click(byLabel("Invert the pages (for a dark theme)"));
        show("inverted");
        fireEvent.click(byLabel("Show the pages as they are"));
        show("back");
        await snapshot("inverts");
    });

    it("sends external links to a browser pane and drops the rest", async () => {
        await ready();
        const page = q(".pdfViewer");
        const link = (href: string, extra?: (a: HTMLAnchorElement) => void) => {
            const a = document.createElement("a");
            a.setAttribute("href", href);
            extra?.(a);
            const inner = document.createElement("span");
            a.appendChild(inner);
            page.appendChild(a);
            const ev = new MouseEvent("click", { bubbles: true, cancelable: true });
            inner.dispatchEvent(ev);
            log(`click ${href}: prevented=${ev.defaultPrevented}`);
        };
        link("https://example.com/x");
        link("http://example.com/y");
        link("mailto:a@b.c");
        link("javascript:alert(1)");
        link("#page=3");
        link("");
        const plain = document.createElement("div");
        page.appendChild(plain);
        const ev = new MouseEvent("click", { bubbles: true, cancelable: true });
        plain.dispatchEvent(ev);
        log(`click plain: prevented=${ev.defaultPrevented}`);
        await snapshot("links");
    });

    it("asks for a password and tries again", async () => {
        h.manual = true;
        mount();
        await settle();
        const task = h.tasks[0];
        task.onPassword((pw: string) => log(`sendPassword ${pw}`), 1);
        await settle();
        show("asks");
        log(`focus: ${focusName()}`);
        const pw = byLabel("PDF password") as HTMLInputElement;
        pw.value = "wrong";
        fireEvent.submit(pw.closest("form")!);
        show("submitted");
        task.onPassword((pw2: string) => log(`sendPassword again ${pw2}`), 2);
        await settle();
        show("wrong password");
        const again = byLabel("PDF password") as HTMLInputElement;
        again.value = "right";
        fireEvent.submit(again.closest("form")!);
        task.resolve();
        await settle();
        show("opened");
        await snapshot("password");
    });

    it("shows why a file can't be opened", async () => {
        h.manual = true;
        const m = mount();
        await settle();
        h.tasks[0].reject(new Error("Invalid PDF structure."));
        await settle();
        show("an Error");
        m.setRevision(2);
        await settle();
        show("reloading");
        h.tasks[1].reject("plain text failure");
        await settle();
        show("a string");
        await snapshot("error");
    });

    it("shows a failed file read", async () => {
        h.blobImpl = async () => {
            throw new Error("HTTP 404");
        };
        mount();
        await settle();
        show("read failed");
        await snapshot("read-error");
    });

    it("opens a dropped file from its bytes", async () => {
        const file = { size: 2048, arrayBuffer: async () => new ArrayBuffer(2048) } as unknown as File;
        mount({ path: "", file, size: undefined });
        await settle();
        h.bus!.emit("pagesinit");
        show("dropped file");
        await snapshot("dropped-file");
    });

    it("reads a large file in ranges", async () => {
        mount({ size: WHOLE_FILE_MAX_BYTES + 1 });
        await settle();
        h.bus!.emit("pagesinit");
        show("ready");
        h.range.requestDataRange(65536, 131072);
        await settle();
        h.rangeImpl = async () => {
            throw new Error("HTTP 500");
        };
        h.range.requestDataRange(0, 100);
        await settle();
        show("a range failed");
        await snapshot("ranges");
    });

    it("stops delivering ranges once the viewer has moved on", async () => {
        const m = mount({ size: WHOLE_FILE_MAX_BYTES + 1 });
        await settle();
        const range = h.range;
        h.rangeImpl = async () => {
            throw new Error("late failure");
        };
        m.setRevision(2);
        await settle();
        range.requestDataRange(0, 10);
        await settle();
        show("old range failed after reload");
        await snapshot("stale-range");
    });

    it("falls back to the whole file when the server won't send ranges", async () => {
        h.rangeImpl = async () => ({ blob: null, total: 0 });
        mount({ size: WHOLE_FILE_MAX_BYTES + 1 });
        await settle();
        show("ready");
        await snapshot("range-fallback");
    });

    it("reloads a changed file and keeps the page", async () => {
        const m = await ready();
        fireEvent.click(byLabel("Next page"));
        fireEvent.click(byLabel("Next page"));
        fireEvent.click(byLabel("Next page"));
        fireEvent.click(byLabel("Next page"));
        show("on page 5");
        h.docPages = 3;
        m.setRevision(2);
        show("reloading");
        await settle();
        h.bus!.emit("pagesinit");
        show("reloaded with fewer pages");
        m.setPath("/docs/b.pdf");
        await settle();
        h.bus!.emit("pagesinit");
        show("another file");
        const file = { size: 10, arrayBuffer: async () => new ArrayBuffer(10) } as unknown as File;
        m.setFile(file);
        await settle();
        show("a dropped file");
        await snapshot("reloads");
    });

    it("drops a load that was superseded", async () => {
        h.manual = true;
        const m = mount();
        await settle();
        m.setRevision(2);
        await settle();
        m.setRevision(3);
        await settle();
        h.tasks[0].resolve();
        await settle();
        h.tasks[1].reject(new Error("too late"));
        await settle();
        show("old ones finished");
        h.tasks[2].resolve();
        await settle();
        show("newest finished");
        await snapshot("superseded");
    });

    it("drops a load that is superseded while the runtime loads", async () => {
        const m = mount();
        m.setRevision(2);
        await settle();
        show("second load");
        await snapshot("superseded-early");
    });

    it("lets go of everything when closed, and the shared signal outlives the first viewer", async () => {
        const first = await ready();
        const second = render(() => (
            <PdfViewer
                path="/docs/c.pdf"
                revision={1}
                blockId="blk-2"
                zoom={() => 1}
                bar={{ viewer: "pdf", options: ["pdf"], onReopenWith: () => {} }}
                onOpenLink={() => {}}
            />
        ));
        await settle();
        log(`signals: ${h.signals.map((s) => s.aborted).join(",")}`);
        first.view.unmount();
        log(`after the first closed: ${h.signals.map((s) => s.aborted).join(",")}`);
        second.unmount();
        log(`after the last closed: ${h.signals.map((s) => s.aborted).join(",")}`);
        h.bus!.emit("pagesinit");
        await snapshot("closes");
    });

    it("counts the pages of a one page file", async () => {
        h.docPages = 1;
        await ready();
        show("one page");
        await snapshot("one-page");
    });

    it("drops a load that is superseded while the file is read", async () => {
        let release!: () => void;
        const slow = new Promise<void>((r) => (release = r));
        let first = true;
        h.blobImpl = async () => {
            if (first) {
                first = false;
                await slow;
            }
            return { arrayBuffer: async () => new ArrayBuffer(8) };
        };
        const m = mount();
        await settle();
        m.setRevision(2);
        await settle();
        release();
        await settle();
        show("only the second load opened");
        await snapshot("superseded-read");
    });

    it("closes while still loading", async () => {
        h.manual = true;
        const m = mount();
        await settle();
        m.view.unmount();
        h.tasks[0].resolve();
        await settle();
        log("done");
        await snapshot("closes-loading");
    });
});
