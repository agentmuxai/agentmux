// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The strip's measured layout, pinned before the component is split: the real
 * widths its observers report decide how many rows there are, which slots
 * share a row, where the stats zone sits and what a measurement is rounded
 * to. jsdom has no layout, so a fake ResizeObserver delivers the strip width
 * and `getBoundingClientRect` answers from a table of widths per element.
 * Row-layout maths itself is in composer-row-layout.test.ts; the unmeasured
 * fallback and the slots' content are in AgentComposerStrip.test.tsx.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentComposerStrip } from "./AgentComposerStrip";

vi.mock("../runtime-apply", () => ({ applyRuntimeChange: vi.fn().mockResolvedValue(undefined) }));

type ROCallback = (entries: ResizeObserverEntry[]) => void;
interface FakeRecord {
    cb: ROCallback;
    targets: Set<Element>;
    disconnected: boolean;
}
let observers: FakeRecord[] = [];
class FakeResizeObserver {
    private rec: FakeRecord;
    constructor(cb: ROCallback) {
        this.rec = { cb, targets: new Set(), disconnected: false };
        observers.push(this.rec);
    }
    observe(el: Element): void {
        this.rec.targets.add(el);
    }
    unobserve(el: Element): void {
        this.rec.targets.delete(el);
    }
    disconnect(): void {
        this.rec.targets.clear();
        this.rec.disconnected = true;
    }
}

/** The width each measured element reports, by what it is. The rows element reports the strip width. */
const widths: Record<string, number> = {};
let rowsWidth = 0;
let outerWidth = 0;
let columnGap = "0px";
let spaceTwo = "";
const SELECTORS: [string, string][] = [
    [".agent-runtime-dropup-trigger", "runtime"],
    [".agent-composer-strip-auth", "auth"],
    [".agent-session-stats-trigger", "ctxReading"],
    [".agent-composer-strip-compact-btn", "compact"],
    [".agent-composer-strip-log-btn", "shell"],
    [".runtime-badge", "badge"],
    [".agent-composer-strip-view", "view"],
];
function widthOf(el: Element): number {
    if (el.classList.contains("agent-composer-strip-rows")) return rowsWidth;
    if (el.classList.contains("agent-composer-strip")) return outerWidth;
    for (const [selector, name] of SELECTORS) if (el.matches(selector)) return widths[name] ?? 0;
    return 0;
}

const realRect = Element.prototype.getBoundingClientRect;
const realComputedStyle = window.getComputedStyle;

beforeEach(() => {
    observers = [];
    for (const k of Object.keys(widths)) delete widths[k];
    Object.assign(widths, { runtime: 120, auth: 100, ctxReading: 90, compact: 70, shell: 50, badge: 60 });
    rowsWidth = 0;
    outerWidth = 100;
    columnGap = "0px";
    spaceTwo = "";
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    Element.prototype.getBoundingClientRect = function (this: Element): DOMRect {
        const w = widthOf(this);
        return { width: w, height: 20, top: 0, left: 0, right: w, bottom: 20, x: 0, y: 0, toJSON() {} } as DOMRect;
    };
    window.getComputedStyle = (() =>
        ({ columnGap, getPropertyValue: () => spaceTwo }) as unknown as CSSStyleDeclaration) as typeof window.getComputedStyle;
});

afterEach(() => {
    cleanup();
    Element.prototype.getBoundingClientRect = realRect;
    window.getComputedStyle = realComputedStyle;
    vi.unstubAllGlobals();
});

const baseProps = {
    logOpen: false,
    onToggleLog: () => {},
    providerId: "claude",
    blockId: "b1",
    blockAtom: () => ({ meta: {} }) as never,
    authStatus: "authenticated" as const,
    contextTokens: 50_000,
    contextWindow: 200_000,
    onCompact: () => {},
    agentMode: "host",
};

function renderStrip(extra: Partial<Parameters<typeof AgentComposerStrip>[0]> = {}) {
    const utils = render(() => <AgentComposerStrip {...baseProps} {...extra} />);
    const strip = utils.container.querySelector(".agent-composer-strip") as HTMLElement;
    // The strip's own layout width, which the zoom ratio divides the rect by.
    Object.defineProperty(strip, "clientWidth", { configurable: true, value: 100 });
    return { ...utils, strip };
}

function slotKey(slot: Element): string {
    if (slot.querySelector(".agent-composer-strip-log-btn")) return "hostShell";
    if (slot.querySelector(".agent-runtime-dropup-trigger")) return "runtime";
    if (slot.querySelector(".agent-composer-strip-auth")) return "auth";
    if (slot.querySelector(".agent-composer-strip-view")) return "view";
    return "ctx";
}
function layout(container: HTMLElement): { left: string[]; right: string[] }[] {
    const keys = (row: Element, side: string) =>
        [...row.querySelectorAll(`:scope > .agent-composer-strip-row-${side} > .agent-composer-strip-slot-measure`)].map(slotKey);
    return [...container.querySelectorAll(".agent-composer-strip-row")].map((row) => ({ left: keys(row, "left"), right: keys(row, "right") }));
}

/** The observer reports a new width for the rows element, as the browser would after a layout. */
function resizeTo(container: HTMLElement, width: number): void {
    rowsWidth = width;
    const rows = container.querySelector(".agent-composer-strip-rows")!;
    for (const o of observers) if (o.targets.has(rows)) o.cb([{ target: rows } as ResizeObserverEntry]);
}

const ONE_ROW = [{ left: ["runtime", "auth"], right: ["ctx", "hostShell"] }];
const TWO_ROWS = [
    { left: ["auth"], right: ["ctx"] },
    { left: ["runtime"], right: ["hostShell"] },
];

/** The narrowest strip, in px, on which everything shares one row. */
function narrowestOneRow(container: HTMLElement): number {
    for (let w = 100; w <= 900; w++) {
        resizeTo(container, w);
        if (layout(container).length === 1) return w;
    }
    return -1;
}

describe("AgentComposerStrip — measured row layout", () => {
    it("shows the fixed-side single row until the strip has reported a width", () => {
        const { container } = renderStrip();
        expect(layout(container)).toEqual(ONE_ROW);
    });

    it("keeps everything on one row when the slots and their gaps fit", () => {
        const { container } = renderStrip();
        resizeTo(container, 1000);
        expect(layout(container)).toEqual(ONE_ROW);
    });

    it("splits into two balanced rows, each with content on both sides, when they don't", () => {
        const { container } = renderStrip();
        resizeTo(container, 400);
        expect(layout(container)).toEqual(TWO_ROWS);
    });

    it("splits into three rows, one slot alone on a side, in a very narrow strip", () => {
        const { container } = renderStrip();
        resizeTo(container, 150);
        expect(layout(container)).toEqual([
            { left: ["auth"], right: [] },
            { left: [], right: ["ctx"] },
            { left: ["runtime"], right: ["hostShell"] },
        ]);
    });

    it("is exactly as wide as the slots it rounds up and the gaps between them: 120 + 104 + 160 + 112 + 3 x 8", () => {
        const { container } = renderStrip();
        expect(narrowestOneRow(container)).toBe(520);
        resizeTo(container, 519);
        expect(layout(container)).toEqual(TWO_ROWS);
    });

    it("rounds each slot's width up to the next 8 px, never to the nearest", () => {
        const { container } = renderStrip();
        widths.auth = 97;
        expect(narrowestOneRow(container)).toBe(520);
        widths.auth = 96;
        expect(narrowestOneRow(container)).toBe(512);
    });

    it("adds the column gap between the children of a multi-child slot", () => {
        columnGap = "4px";
        const { container } = renderStrip();
        expect(narrowestOneRow(container)).toBe(536);
    });

    it("scales the gaps by the strip's zoom (its rect over its layout width)", () => {
        columnGap = "4px";
        outerWidth = 200;
        const { container } = renderStrip();
        expect(narrowestOneRow(container)).toBe(560);
    });

    it("scales the gap inside a multi-child slot by the zoom as well, not only the gap between slots", () => {
        columnGap = "10px";
        outerWidth = 200;
        const { container } = renderStrip();
        // 120 + 104 + (160 + 20 -> 184) + (110 + 20 -> 136) = 544, plus 3 gaps of 8 x 2.
        expect(narrowestOneRow(container)).toBe(592);
    });

    it("reads the gap between a row's children from --space-2, 8 px when it can't", () => {
        spaceTwo = "12";
        const { container } = renderStrip();
        expect(narrowestOneRow(container)).toBe(532);
    });

    it("measures again when only the strip width changes, so a slot a container query hid drops out", () => {
        const { container } = renderStrip();
        resizeTo(container, 450);
        expect(layout(container)).toEqual(TWO_ROWS);
        widths.auth = 0;
        resizeTo(container, 451);
        expect(layout(container)).toEqual([{ left: ["runtime", "auth"], right: ["ctx", "hostShell"] }]);
    });

    it("keeps a slot that measures 0 px out of the pairing and folds it into the last row on its own side", () => {
        widths.auth = 0;
        const { container } = renderStrip();
        resizeTo(container, 300);
        expect(layout(container)).toEqual([
            { left: ["ctx"], right: [] },
            { left: ["runtime", "auth"], right: ["hostShell"] },
        ]);
    });
});

describe("AgentComposerStrip — a hidden slot", () => {
    it("on the right folds into the last row's right side", () => {
        widths.ctxReading = 0;
        widths.compact = 0;
        const { container } = renderStrip();
        resizeTo(container, 300);
        expect(layout(container)).toEqual([
            { left: ["auth"], right: [] },
            { left: ["runtime"], right: ["hostShell", "ctx"] },
        ]);
    });
});

describe("AgentComposerStrip — where the stats zone sits", () => {
    it("shares the single row's line when everything fits on it", () => {
        const { container, strip } = renderStrip();
        resizeTo(container, 1000);
        expect(strip.querySelectorAll(".agent-composer-strip-stats-zone").length).toBe(1);
        expect(strip.querySelector(":scope > .agent-composer-strip-stats-zone")).toBeNull();
        expect(container.querySelector(".agent-composer-strip-rows .agent-composer-strip-row > .agent-composer-strip-stats-zone")).not.toBeNull();
    });

    it("moves above the rows once the strip needs more than one", () => {
        const { container, strip } = renderStrip();
        resizeTo(container, 400);
        expect(strip.querySelectorAll(".agent-composer-strip-stats-zone").length).toBe(1);
        expect(strip.querySelector(":scope > .agent-composer-strip-stats-zone")).not.toBeNull();
        expect(strip.firstElementChild?.classList.contains("agent-composer-strip-stats-zone")).toBe(true);
    });
});

describe("AgentComposerStrip — the anchors stay put", () => {
    it("keeps the runtime trigger and the Shell button the same elements when the row count changes", () => {
        const { container } = renderStrip();
        resizeTo(container, 1000);
        const runtime = container.querySelector(".agent-runtime-dropup-trigger");
        const shell = container.querySelector(".agent-composer-strip-log-btn");
        expect(runtime).not.toBeNull();
        resizeTo(container, 400);
        expect(layout(container).length).toBe(2);
        expect(container.querySelector(".agent-runtime-dropup-trigger")).toBe(runtime);
        expect(container.querySelector(".agent-composer-strip-log-btn")).toBe(shell);
        resizeTo(container, 150);
        expect(layout(container).length).toBe(3);
        expect(container.querySelector(".agent-runtime-dropup-trigger")).toBe(runtime);
        expect(container.querySelector(".agent-composer-strip-log-btn")).toBe(shell);
    });

    it("puts the anchors in the last row, however many rows there are", () => {
        const { container } = renderStrip();
        for (const w of [1000, 400, 150]) {
            resizeTo(container, w);
            const rows = layout(container);
            const last = rows[rows.length - 1];
            expect([...last.left, ...last.right]).toEqual(expect.arrayContaining(["runtime", "hostShell"]));
        }
    });
});

describe("AgentComposerStrip — the observer", () => {
    it("watches the rows element and lets go of it on unmount", () => {
        const { container, unmount } = renderStrip();
        const rows = container.querySelector(".agent-composer-strip-rows")!;
        expect(observers.some((o) => o.targets.has(rows))).toBe(true);
        unmount();
        expect(observers.every((o) => o.disconnected)).toBe(true);
    });
});
