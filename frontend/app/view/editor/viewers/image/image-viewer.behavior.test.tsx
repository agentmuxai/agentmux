// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The image viewer, pinned as a whole ahead of splitting it into a state object and view parts: each scenario
 * drives it (loading and failing, the toolbar, the facts, the background, the wheel, clicks, drags and keys, the
 * zoom-out modifier, a new image, a resized pane, unmounting) and logs it as a tree and, in order, what it asks of
 * its byte loader, its bar and its listeners. The logs are snapshots recorded on the unsplit component; the split
 * must leave them byte for byte. (image-viewer.test.tsx asserts the click zoom; this pins the markup and the rest.)
 */

import { setPlatform } from "@/util/platformutil";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createEffect, createSignal } from "solid-js";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    log: [] as string[],
    setUrl: (_: string | null) => {},
    setErr: (_: string) => {},
    initialUrl: "blob:image" as string | null,
    resizeCallbacks: [] as (() => void)[],
}));

// The byte loader, with its source logged whenever it changes.
vi.mock("../file-bytes", () => ({
    createFileBytes: (source: () => unknown) => {
        const [url, setUrl] = createSignal<string | null>(h.initialUrl);
        const [error, setError] = createSignal("");
        h.setUrl = setUrl;
        h.setErr = setError;
        createEffect(() => h.log.push(`bytes source ${JSON.stringify(source())}`));
        return {
            url,
            error,
            setError: (m: string) => {
                h.log.push(`bytes setError ${JSON.stringify(m)}`);
                setError(m);
            },
        };
    },
}));
// The bar, as a stand-in that shows what it is given.
vi.mock("../viewer-bar", () => ({
    ViewerBar: (props: any) => {
        const { start, facts, ...rest } = props;
        h.log.push(`bar props ${JSON.stringify(Object.keys(rest))} ${JSON.stringify(rest.options)} viewer=${rest.viewer}`);
        return (
            <div class="bar" data-facts={JSON.stringify(facts)}>
                {start}
            </div>
        );
    },
}));

import { ImageViewer, type ImageViewerProps } from "./image-viewer";

const T = h.log;
const log = (line: string) => T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/image-viewer.behavior.${name}.txt`);

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|href|style|tabindex|alt|src|draggable|aria-pressed)$/;
/** The viewer as an indented tree: tags, classes, the attributes that carry state, and text. */
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

// A 2000×1000 image fitted into an 800×600 view: 800×400 on screen.
const VIEW = { w: 800, h: 600 };
const FITTED = { w: 800, h: 400 };
const NATURAL = { w: 2000, h: 1000 };

function size(el: Element, props: Record<string, number>) {
    for (const [k, v] of Object.entries(props)) Object.defineProperty(el, k, { configurable: true, value: v });
}

let host: HTMLElement;
let content: HTMLElement;
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n"));
};
const img = () => host.querySelector<HTMLImageElement>("img")!;
const baseProps = (): ImageViewerProps => ({
    path: "C:/pics/a.png",
    mime: "image/png",
    size: 2048,
    revision: 0,
    bar: { viewer: "image", options: ["image", "text"], onReopenWith: () => {} } as any,
});

/** Gives the content and the image their sizes, as a browser would. */
function layout(natural = NATURAL) {
    size(content, { offsetWidth: VIEW.w, offsetHeight: VIEW.h });
    content.getBoundingClientRect = () => ({ left: 0, top: 0, width: VIEW.w, height: VIEW.h }) as DOMRect;
    if (img()) size(img(), { offsetWidth: FITTED.w, offsetHeight: FITTED.h, naturalWidth: natural.w, naturalHeight: natural.h });
}
/** Mounts it, lays it out and, unless told otherwise, lets the image decode. */
function mount(over: Partial<ImageViewerProps> = {}, opts: { decode?: boolean; url?: string | null } = {}) {
    h.initialUrl = opts.url === undefined ? "blob:image" : opts.url;
    const props = { ...baseProps(), ...over };
    const view = render(() => <ImageViewer {...props} />);
    host = view.container;
    content = host.querySelector<HTMLElement>(".image-viewer-content")!;
    layout();
    if (opts.decode !== false && img()) fireEvent.load(img());
    return view;
}

const press = (x = 400, y = 300, extra: Record<string, unknown> = {}) =>
    fireEvent.pointerDown(img(), { button: 0, buttons: 1, pointerId: 1, clientX: x, clientY: y, ...extra });
const move = (x: number, y: number, extra: Record<string, unknown> = {}) =>
    fireEvent.pointerMove(img(), { buttons: 1, pointerId: 1, clientX: x, clientY: y, ...extra });
const release = (x = 400, y = 300, extra: Record<string, unknown> = {}) =>
    fireEvent.pointerUp(img(), { button: 0, buttons: 0, pointerId: 1, clientX: x, clientY: y, ...extra });
const click = (extra: Record<string, unknown> = {}, x = 400, y = 300) => {
    press(x, y, extra);
    release(x, y, extra);
};
const key = (k: string, extra: Record<string, unknown> = {}) => {
    const notPrevented = fireEvent.keyDown(content, { key: k, ...extra });
    log(`key ${JSON.stringify(k)}${Object.keys(extra).length ? " " + JSON.stringify(extra) : ""} prevented=${!notPrevented}`);
};
const button = (name: string) => [...host.querySelectorAll("button")].find((b) => b.textContent?.trim() === name || b.getAttribute("aria-label") === name || b.title === name)!;

beforeAll(() => {
    Element.prototype.setPointerCapture ??= () => {};
});
beforeEach(() => {
    T.length = 0;
    h.resizeCallbacks.length = 0;
    setPlatform("win32");
    vi.stubGlobal(
        "ResizeObserver",
        class {
            constructor(cb: () => void) {
                h.resizeCallbacks.push(cb);
                log("ResizeObserver created");
            }
            observe() {
                log("ResizeObserver observe");
            }
            disconnect() {
                log("ResizeObserver disconnect");
            }
        },
    );
});
afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
});

describe("Image viewer: loading and the facts", () => {
    it("shows Loading… until the bytes arrive, then the image faded out until it decodes", () => {
        mount({}, { url: null, decode: false });
        look("no bytes yet");
        h.setUrl("blob:one");
        layout();
        look("bytes arrived, not decoded");
        fireEvent.load(img());
        look("decoded");
        h.setUrl("blob:two");
        look("a new image, not decoded");
        return snapshot("loading");
    });

    it("shows why the bytes can't be read, and when the image can't be displayed", () => {
        mount({}, { url: null, decode: false });
        h.setErr("Couldn't read the file.");
        look("read failed");
        cleanup();
        mount({}, { decode: false });
        fireEvent.error(img());
        look("display failed");
        return snapshot("errors");
    });

    it("lists the size in pixels, the byte size and the type, and the bar gets what the caller gave it", () => {
        mount();
        look("png");
        for (const [mime, size] of [
            ["image/svg+xml", 10],
            ["image/x-icon", undefined],
            ["image/jpeg", 0],
            ["", 5],
        ] as const) {
            cleanup();
            mount({ mime, size, path: mime === "image/svg+xml" ? "C:/pics/a.svg" : "C:/pics/a.png" });
            log(`-- ${mime || "no mime"} size=${size}`);
            log(`facts ${host.querySelector(".bar")!.getAttribute("data-facts")}`);
        }
        cleanup();
        mount({ path: "", size: undefined, file: new File(["abc"], "dropped.svg", { type: "image/svg+xml" }) });
        log("-- dropped file");
        log(`facts ${host.querySelector(".bar")!.getAttribute("data-facts")}`);
        look("dropped file");
        return snapshot("facts");
    });

    it("hands the loader the path, the dropped file, the revision and the SVG type", () => {
        mount({ path: "C:/pics/a.svg", mime: "text/plain" });
        cleanup();
        mount({ path: "", file: new File(["x"], "d.png"), revision: 3 });
        return snapshot("source");
    });
});

describe("Image viewer: toolbar and background", () => {
    it("starts on a checkerboard, or dark for a JPEG or BMP, and B or the button cycles through all three", () => {
        mount();
        look("png");
        key("b");
        look("after b");
        key("B");
        look("after B");
        fireEvent.click(button("Background: light (B)"));
        look("after the button");
        for (const mime of ["image/jpeg", "image/bmp", "image/svg+xml", "image/gif"]) {
            cleanup();
            mount({ mime });
            log(`-- ${mime}: ${content.className}`);
        }
        return snapshot("background");
    });

    it("Fit and 100% zoom; the percent and the Fit badge show once zoomed", () => {
        mount();
        look("fit");
        fireEvent.click(button("Actual size (1)"));
        look("100%");
        fireEvent.click(button("Fit to the pane (0)"));
        look("fit again");
        click();
        look("zoomed in a step");
        fireEvent.click(button("Fit to the pane (0)"));
        look("fit from the toolbar");
        return snapshot("toolbar");
    });
});

describe("Image viewer: wheel, clicks and drags", () => {
    it("the wheel zooms about the cursor, but not with Ctrl or Cmd, nor before the image has decoded", () => {
        mount({}, { decode: false });
        const wheel = (label: string, extra: Record<string, unknown>) => {
            const notPrevented = fireEvent.wheel(content, { deltaY: -100, deltaMode: 0, clientX: 600, clientY: 200, ...extra });
            log(`wheel ${label} prevented=${!notPrevented}`);
            log(img().style.transform);
        };
        wheel("before decode", {});
        fireEvent.load(img());
        wheel("in at the top right", {});
        wheel("out at the same spot", { deltaY: 100 });
        wheel("ctrl", { ctrlKey: true });
        wheel("meta", { metaKey: true });
        wheel("in, by lines", { deltaMode: 1, deltaY: -3 });
        look("after");
        return snapshot("wheel");
    });

    it("a click zooms a step in about the pointer, Ctrl or Alt out, and a macOS Ctrl+click is left alone", () => {
        mount();
        click({}, 600, 200);
        log(img().style.transform);
        click({}, 100, 500);
        log(img().style.transform);
        look("two steps in");
        click({ altKey: true });
        log(img().style.transform);
        click({ ctrlKey: true });
        log(img().style.transform);
        setPlatform("darwin");
        click({ ctrlKey: true });
        log(`mac ctrl+click: ${img().style.transform}`);
        click({ altKey: true });
        log(`mac alt+click: ${img().style.transform}`);
        click({}, 600, 200);
        for (let i = 0; i < 6; i++) click();
        look("at the deepest step");
        return snapshot("click");
    });

    it("a drag pans a zoomed image, shows the grab cursor, and ends on release, cancel, or a move with no button", () => {
        mount();
        click();
        click();
        look("zoomed");
        press(400, 300);
        move(403, 300);
        log(`3 px: ${img().style.transform}`);
        move(380, 280);
        look("dragging");
        release(380, 280);
        look("released");
        press(400, 300);
        move(350, 250);
        fireEvent.pointerCancel(img(), { pointerId: 1 });
        look("cancelled");
        press(400, 300);
        move(360, 260);
        move(300, 200, { buttons: 0 });
        look("moved with no button");
        move(100, 100);
        log(`a move after it ended: ${img().style.transform}`);
        fireEvent.pointerDown(img(), { button: 2, buttons: 2, pointerId: 1, clientX: 400, clientY: 300 });
        move(200, 200);
        log(`right button: ${img().style.transform}`);
        return snapshot("drag");
    });

    it("a drag at fit does nothing, and a press that drags does not zoom when released", () => {
        mount();
        press();
        move(450, 300);
        release(450, 300);
        log(`at fit: ${img().style.transform}`);
        click();
        press();
        move(430, 300);
        release(430, 300);
        look("dragged, not clicked");
        return snapshot("drag-at-fit");
    });

    it("the zoom-out cursor follows Ctrl, Alt (and on macOS only Alt) held, through the window and the pointer, and blur", () => {
        mount();
        const cursor = (label: string) => log(`${label}: zoom-out=${img().classList.contains("image-viewer-zoom-out")}`);
        fireEvent.keyDown(window, { key: "Alt", altKey: true });
        cursor("alt down");
        fireEvent.keyUp(window, { key: "Alt", altKey: false });
        cursor("alt up");
        fireEvent.keyDown(window, { key: "Control", ctrlKey: true });
        cursor("ctrl down");
        fireEvent.blur(window);
        cursor("blur");
        fireEvent.pointerMove(img(), { ctrlKey: true, clientX: 1, clientY: 1 });
        cursor("pointer moved with ctrl");
        fireEvent.pointerMove(img(), { clientX: 1, clientY: 1 });
        cursor("pointer moved without");
        setPlatform("darwin");
        fireEvent.keyDown(window, { key: "Control", ctrlKey: true });
        cursor("mac ctrl down");
        fireEvent.keyDown(window, { key: "Alt", altKey: true });
        cursor("mac alt down");
        press(400, 300, { altKey: false });
        cursor("mac press without");
        return snapshot("zoom-out-cursor");
    });
});

describe("Image viewer: keys", () => {
    it("+ - = _ zoom about the centre, 0 fits, 1 shows actual size, and the arrows pan only when zoomed", () => {
        mount();
        key("ArrowLeft");
        log(img().style.transform);
        for (const k of ["+", "=", "+", "-", "_"]) {
            key(k);
            log(img().style.transform);
        }
        for (const k of ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "ArrowLeft"]) {
            key(k);
            log(img().style.transform);
        }
        key("1");
        log(img().style.transform);
        key("0");
        log(img().style.transform);
        key("x");
        key("Tab");
        look("end");
        return snapshot("keys");
    });

    it("[ and ] step through the folder and stop there, B works before the image decodes, the rest waits for it, Ctrl, Cmd and Alt are left alone", () => {
        const steps: number[] = [];
        mount({ onStep: (d) => (steps.push(d), log(`onStep ${d}`)) }, { decode: false });
        host.addEventListener("keydown", () => log("parent saw keydown"));
        key("[");
        key("]");
        key("b");
        key("+");
        key("0");
        key("]", { ctrlKey: true });
        key("]", { metaKey: true });
        key("b", { altKey: true });
        log(`steps ${JSON.stringify(steps)}`);
        cleanup();
        mount();
        host.addEventListener("keydown", () => log("parent saw keydown (no onStep)"));
        key("[");
        key("+");
        return snapshot("keys-folder");
    });
});

describe("Image viewer: a new image, a resized pane, unmounting", () => {
    it("a newer render of the same size keeps the zoom, another size goes back to fit", () => {
        mount();
        click();
        click();
        log(`zoomed: ${img().style.transform}`);
        h.setUrl("blob:same-size");
        look("a newer render, not decoded");
        fireEvent.load(img());
        log(`same size decoded: ${img().style.transform}`);
        h.setUrl("blob:other-size");
        layout({ w: 640, h: 480 });
        fireEvent.load(img());
        log(`other size decoded: ${img().style.transform}`);
        look("end");
        return snapshot("new-image");
    });

    it("a resized pane keeps a zoomed image in reach, and does nothing at fit", () => {
        mount();
        h.resizeCallbacks[0]();
        log(`at fit: ${img().style.transform}`);
        click();
        click();
        click();
        key("ArrowLeft");
        log(img().style.transform);
        size(content, { offsetWidth: 300, offsetHeight: 200 });
        content.getBoundingClientRect = () => ({ left: 0, top: 0, width: 300, height: 200 }) as DOMRect;
        h.resizeCallbacks[0]();
        log(`resized: ${img().style.transform}`);
        return snapshot("resize");
    });

    it("without a ResizeObserver it still works", () => {
        vi.stubGlobal("ResizeObserver", undefined);
        mount();
        click();
        look("zoomed");
        return snapshot("no-resize-observer");
    });

    it("listens on the window and the content while mounted, and lets go when unmounted", () => {
        const win = vi.spyOn(window, "addEventListener");
        const winOff = vi.spyOn(window, "removeEventListener");
        const view = mount();
        const wheel = vi.spyOn(content, "removeEventListener");
        for (const [name, c] of [["window add", win.mock.calls], ["window remove", winOff.mock.calls]] as const) {
            log(`${name}: ${JSON.stringify(c.map((a) => a[0]).filter((t) => ["keydown", "keyup", "blur"].includes(String(t))))}`);
        }
        view.unmount();
        log(`window remove after: ${JSON.stringify(winOff.mock.calls.map((a) => a[0]).filter((t) => ["keydown", "keyup", "blur"].includes(String(t))))}`);
        log(`content remove: ${JSON.stringify(wheel.mock.calls.map((a) => [a[0]]))}`);
        return snapshot("listeners");
    });
});
