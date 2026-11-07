// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * LogView (the install dialog's read-only console), pinned as a whole ahead of splitting it into a state object and
 * views: each scenario mounts the real log with stand-ins for the follow-scroll machine, the context menu, the
 * clipboard, secret redaction, the animation frame and the resize observer, over a layout whose size the scenario sets,
 * grows and scrolls the log, and logs it as an indented tree (inline styles included) and, in order, every call it
 * makes (the follow machine created, told of each scroll and asked about each new line, each frame, each menu item
 * and copy). The ANSI parsing is the real one.
 * The logs are snapshots recorded on the unsplit component; the split must leave them byte for byte.
 * (LogView.test.tsx asserts the windowing and the menu against the real follow machine; this pins the markup and calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({
    opts: null as any,
    observed: true,
    windowOpen: false,
    pointerHeld: false,
    clipboardFails: false,
    menu: null as any,
    frames: [] as (() => void)[],
    resize: null as any,
}));

vi.mock("@/app/element/ui", () => ({
    openMenuAt: (items: any[], e: Event) => {
        log(`openMenuAt ${JSON.stringify(items.map((i) => [i.label, i.icon, i.enabled]))} prevented=${e.defaultPrevented}`);
        h.menu = items;
    },
}));
vi.mock("@/app/util/menu-icons", () => ({ MENU_ICON: { copy: "icon-copy" } }));
vi.mock("@/app/errors/redact", () => ({ redactSecrets: (s: string) => `redacted(${s})` }));
vi.mock("@/util/clipboard", () => ({
    writeText: (text: string) => (log(`writeText ${JSON.stringify(text)}`), h.clipboardFails ? Promise.reject(new Error("no clipboard")) : Promise.resolve()),
}));
vi.mock("@/app/scroll/follow-scroll", () => ({
    createFollowScroll: (_el: HTMLElement, opts: any) => {
        log(`createFollowScroll ${Object.keys(opts).join(",")}`);
        h.opts = opts;
        return {
            observed: (kind: string, g: unknown) => (log(`observed ${kind} ${JSON.stringify(g)}`), h.observed),
            noteGeometry: (g: unknown) => log(`noteGeometry ${JSON.stringify(g)}`),
            scrolled: (g: unknown, detach: boolean) => log(`scrolled ${JSON.stringify(g)} detach=${detach}`),
            windowOpen: () => h.windowOpen,
            pointerHeld: () => h.pointerHeld,
            reset: () => log("reset"),
            jumpToNode: (id: string) => log(`jumpToNode ${id}`),
            dispose: () => log("dispose"),
        };
    },
}));

import { LOG_ROW_PX, LogView, type LogViewApi } from "./LogView";
import type { LogLine } from "./install-session";

const SNAP = (name: string) => `./__snapshots__/LogView.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|id|tabindex)$/;

/** The log as an indented tree: tags, classes, the attributes that carry state (styles included), and text. */
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
let unmount: () => void;
let api: LogViewApi;
let el: HTMLElement;
let setLines: (l: readonly LogLine[]) => void;
let setTrimmed: (n: number) => void;

const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
/** A long log is too long to list: how many rows are drawn, the first and the last, and where the window sits. */
const glance = (label: string) => {
    const rows = [...host.querySelectorAll(".install-log-line")].map((r) => r.textContent);
    const win = host.querySelector<HTMLElement>(".install-log-window")!;
    const spacer = host.querySelector<HTMLElement>(".install-log-spacer")!;
    log(`-- ${label}: ${rows.length} rows, first ${JSON.stringify(rows[0])}, last ${JSON.stringify(rows.at(-1))}, window ${win.style.transform}, spacer ${spacer.style.height}, scrollTop ${el.scrollTop}`);
};
const frames = () => {
    const run = h.frames.splice(0);
    log(`frames ${run.length}`);
    run.forEach((cb) => cb());
};
const lines = (n: number, from = 0): LogLine[] => Array.from({ length: n }, (_, i) => ({ text: `line ${from + i}`, tone: "normal" }));
const layout = { client: 0, height: 0 };
const ESC = String.fromCharCode(27);

function mount(initial: LogLine[], trimmed = 0, extra: { apiRef?: boolean } = {}) {
    const [l, sl] = createSignal<readonly LogLine[]>(initial);
    const [t, st] = createSignal(trimmed);
    setLines = sl;
    setTrimmed = st;
    const view = render(
        () => (
            <LogView
                lines={l}
                trimmedLines={t}
                copyAllText={() => "ALL TEXT"}
                apiRef={extra.apiRef === false ? undefined : (a) => (api = a)}
            />
        ),
        { container: document.body.appendChild(document.createElement("div")) }
    );
    host = view.container;
    unmount = view.unmount;
    el = host.querySelector(".install-log") as HTMLElement;
}

beforeEach(() => {
    T.length = 0;
    h.opts = null;
    h.observed = true;
    h.windowOpen = false;
    h.pointerHeld = false;
    h.clipboardFails = false;
    h.menu = null;
    h.frames = [];
    h.resize = null;
    layout.client = 0;
    layout.height = 0;
    Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => layout.client });
    Object.defineProperty(HTMLElement.prototype, "scrollHeight", { configurable: true, get: () => layout.height });
    vi.stubGlobal("requestAnimationFrame", (cb: () => void) => (log("requestAnimationFrame"), h.frames.push(cb)));
    vi.stubGlobal(
        "ResizeObserver",
        class {
            constructor(cb: () => void) {
                h.resize = cb;
            }
            observe() {
                log("ResizeObserver observe");
            }
            disconnect() {
                log("ResizeObserver disconnect");
            }
        }
    );
});
afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    delete (HTMLElement.prototype as any).clientHeight;
    delete (HTMLElement.prototype as any).scrollHeight;
    document.body.innerHTML = "";
});

describe("LogView, pinned", () => {
    it("draws each line as a row with its tone, splits ANSI colours into spans, puts a marker row first when earlier lines were trimmed, sizes the spacer to every row, and draws a window of a long log from a fallback height when the box has no layout yet", () => {
        mount(
            [
                { text: "plain", tone: "normal" },
                { text: `${ESC}[31mred${ESC}[0m then plain`, tone: "error" },
                { text: "$ npm install", tone: "command" },
                { text: "careful", tone: "warning" },
            ],
            0
        );
        look("three lines");
        setTrimmed(4);
        look("four earlier lines trimmed");
        unmount();
        document.body.innerHTML = "";
        mount(lines(500));
        glance("500 lines, no layout");
        layout.client = 320;
        fireEvent.scroll(el);
        glance("after the box has a height of 320");
        return snapshot("rows");
    });

    it("creates the follow machine and a resize observer on mount, remeasures on a resize, tells the machine of each scroll with the geometry and whether a gesture is on, redraws the window as it moves, and disposes the machine and the observer on unmount", () => {
        layout.client = 160;
        layout.height = 8000;
        mount(lines(500));
        log(`machine options: ${h.opts.loadOlder()} ${h.opts.hasOlderHistory()} ${h.opts.loadingOlder()} ${h.opts.transition()}`);
        glance("mounted");
        el.scrollTop = 1600;
        fireEvent.scroll(el);
        glance("scrolled to 1600");
        h.windowOpen = true;
        el.scrollTop = 3200;
        fireEvent.scroll(el);
        glance("scrolled to 3200 in a gesture window");
        h.windowOpen = false;
        h.pointerHeld = true;
        el.scrollTop = 3300;
        fireEvent.scroll(el);
        h.pointerHeld = false;
        layout.client = 480;
        h.resize();
        glance("resized to 480");
        layout.client = 0;
        h.resize();
        glance("resized to nothing keeps the last height");
        unmount();
        log("unmounted");
        return snapshot("machine");
    });

    it("follows new output: a frame after the first draw and after each change in the count of lines or trimmed lines, scrolling to the bottom only if the machine says it is still following, and starting over when the log shrank", () => {
        layout.client = 160;
        layout.height = 800;
        mount(lines(50));
        frames();
        setLines(lines(60));
        layout.height = 960;
        frames();
        h.observed = false;
        setLines(lines(70));
        frames();
        h.observed = true;
        setTrimmed(3);
        layout.height = 1100;
        frames();
        setLines(lines(10));
        frames();
        setLines(lines(10, 100));
        log("-- same count, no frame");
        log(`frames pending ${h.frames.length}`);
        glance("after the shrink");
        setLines(lines(80));
        unmount();
        frames();
        log("-- frame after unmount");
        return snapshot("follow");
    });

    it("hands its parent an api: scrollToLine jumps the machine to the line and moves the window near it (past the marker row when lines were trimmed), and refresh remeasures and follows", () => {
        layout.client = 160;
        layout.height = 8000;
        mount(lines(500));
        frames();
        api.scrollToLine(300);
        glance("scrolled to line 300");
        api.scrollToLine(1);
        glance("scrolled to line 1 (clamped at the top)");
        setTrimmed(2);
        api.scrollToLine(300);
        glance("scrolled to line 300 with a marker row");
        el.scrollTop = 40;
        layout.client = 320;
        api.refresh();
        frames();
        glance("refreshed");
        unmount();
        document.body.innerHTML = "";
        mount(lines(3), 0, { apiRef: false });
        look("no api asked for");
        return snapshot("api");
    });

    it("opens a context menu on right-click with Copy (on only with a selection) and Copy All (on only with lines), copies through the secret redactor, and logs a failed clipboard write", async () => {
        mount(lines(3));
        const selected = vi.spyOn(window, "getSelection");
        selected.mockReturnValue({ toString: () => "picked text" } as Selection);
        fireEvent.contextMenu(el);
        h.menu[0].click();
        h.menu[1].click();
        selected.mockReturnValue({ toString: () => "" } as Selection);
        fireEvent.contextMenu(el);
        selected.mockReturnValue(null);
        fireEvent.contextMenu(el);
        setLines([]);
        fireEvent.contextMenu(el);
        h.clipboardFails = true;
        const spy = vi.spyOn(console, "log").mockImplementation((...a) => void log(`console.log ${a.join(" ")}`));
        selected.mockReturnValue({ toString: () => "again" } as Selection);
        fireEvent.contextMenu(el);
        h.menu[0].click();
        h.menu[1].click();
        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();
        spy.mockRestore();
        selected.mockRestore();
        log(`row height ${LOG_ROW_PX}`);
        return snapshot("menu");
    });
});
