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
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentComposerStrip } from "./AgentComposerStrip";

vi.mock("../runtime-apply", () => ({ applyRuntimeChange: vi.fn().mockResolvedValue(undefined) }));

// The pane's readout: figures there or not, and a plain stand-in for the badge, which draws a tooltip.
const readout = vi.hoisted(() => ({ on: false }));
vi.mock("@/app/hook/useBlockStats", () => ({
    useBlockStats: () => () => (readout.on ? { cpu: 1, mem: 1, peak: 1, pids: 1 } : null),
}));
vi.mock("@/element/blockstats", () => ({
    BlockStatsBadge: (p: { form?: string }) => (
        <span class="block-stats-badge" data-form={p.form}>
            {p.form}
        </span>
    ),
}));

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
/** The width of each hidden probe, by its `data-probe` ("auth:full", "auth:compact"). None: nothing to choose forms from. */
const probeWidths: Record<string, number> = {};
const SELECTORS: [string, string][] = [
    [".agent-runtime-dropup-trigger", "runtime"],
    [".agent-composer-strip-auth", "auth"],
    [".agent-session-stats-trigger", "ctxReading"],
    [".agent-composer-strip-compact-btn", "compact"],
    [".agent-composer-strip-log-btn", "shell"],
    [".runtime-badge", "badge"],
    [".agent-composer-strip-view", "view"],
    [".block-stats-badge", "stats"],
];
function widthOf(el: Element): number {
    const probe = el.getAttribute("data-probe");
    if (probe) return probeWidths[probe] ?? 0;
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
    for (const k of Object.keys(probeWidths)) delete probeWidths[k];
    readout.on = false;
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
    if (slot.querySelector(".block-stats-badge")) return "stats";
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

// Both forms of a slot, as the hidden probes measure them. The full ones match the slot widths above
// (104 + 160 + 112 after rounding), so one row takes 120 + 104 + 160 + 112 + 3 x 8 = 520.
const PROBES = {
    "auth:full": 100,
    "auth:compact": 32,
    "ctx:full": 160,
    "ctx:compact": 112,
    "hostShell:full": 110,
    "hostShell:compact": 32,
};

/** The first match outside the hidden probes, which draw the same elements. */
function real(container: HTMLElement, selector: string): HTMLElement {
    const found = [...container.querySelectorAll<HTMLElement>(selector)].find((el) => !el.closest(".agent-composer-strip-probes"));
    if (!found) throw new Error(`no ${selector} outside the probes`);
    return found;
}

describe("AgentComposerStrip — compact forms", () => {
    const withProbes = () => Object.assign(probeWidths, PROBES);
    const authed = { authEmail: "alice@example.com" };

    it("keeps every slot in its full form while they fit one row", () => {
        withProbes();
        const { container } = renderStrip(authed);
        resizeTo(container, 520);
        expect(layout(container)).toEqual(ONE_ROW);
        expect(real(container, ".agent-composer-strip-auth").textContent).toContain("alice");
        expect(real(container, ".agent-composer-strip-compact-btn").textContent).toBe("Compact");
        expect(real(container, ".agent-composer-strip-log-btn").textContent).toBe("Shell");
    });

    it("shrinks the account first, then the context reading, then Shell, and stays on one row", () => {
        withProbes();
        const { container } = renderStrip(authed);
        const shell = () => real(container, ".agent-composer-strip-log-btn");
        const compactBtn = () => real(container, ".agent-composer-strip-compact-btn");
        const auth = () => real(container, ".agent-composer-strip-auth");

        resizeTo(container, 460);
        expect(layout(container)).toEqual(ONE_ROW);
        expect(auth().textContent).toBe("A");
        expect(compactBtn().textContent).toBe("Compact");
        expect(shell().textContent).toBe("Shell");

        resizeTo(container, 400);
        expect(layout(container)).toEqual(ONE_ROW);
        expect(compactBtn().getAttribute("aria-label")).toBe("Compact");
        expect(compactBtn().textContent).toBe("");
        expect(shell().textContent).toBe("Shell");

        resizeTo(container, 320);
        expect(layout(container)).toEqual(ONE_ROW);
        expect(shell().getAttribute("aria-label")).toBe("Shell");
        expect(shell().textContent).toBe("");
    });

    it("is one row down to 320 px with the compact forms, 520 px without", () => {
        withProbes();
        const { container } = renderStrip(authed);
        expect(narrowestOneRow(container)).toBe(320);
    });

    it("wraps only when even the compact forms don't fit, and keeps the full forms where they cost no row", () => {
        withProbes();
        const { container } = renderStrip(authed);
        resizeTo(container, 300);
        expect(layout(container).length).toBeGreaterThan(1);
        // The compact forms make as many rows as the full ones here, so nothing is shrunk for nothing.
        expect(real(container, ".agent-composer-strip-auth").textContent).toContain("alice");
        expect(real(container, ".agent-composer-strip-log-btn").textContent).toBe("Shell");
    });

    it("uses every compact form that saves a row when one row is out of reach", () => {
        withProbes();
        const { container } = renderStrip(authed);
        resizeTo(container, 280);
        const rows = layout(container).length;
        expect(rows).toBeGreaterThan(1);
        resizeTo(container, 1000);
        expect(layout(container).length).toBe(1);
    });

    it("gives the full forms back when the strip widens", () => {
        withProbes();
        const { container } = renderStrip(authed);
        resizeTo(container, 320);
        resizeTo(container, 1000);
        expect(real(container, ".agent-composer-strip-auth").textContent).toContain("alice");
        expect(real(container, ".agent-composer-strip-log-btn").textContent).toBe("Shell");
    });

    it("marks a signed-out account with ! when it is tight, the full label in its accessible name", () => {
        withProbes();
        const { container } = renderStrip({ authStatus: "unauthenticated" });
        resizeTo(container, 400);
        const auth = real(container, ".agent-composer-strip-auth");
        expect(auth.textContent).toBe("!");
        const named = auth.querySelector("[aria-label]");
        expect(named?.getAttribute("aria-label")).toBe("Not logged in");
        expect(named?.getAttribute("role")).toBe("img");
    });

    it("counts a slot the strip hides (0 px) as taking no room, so the others collapse no earlier than they must", () => {
        withProbes();
        widths.auth = 0;
        const { container } = renderStrip(authed);
        // Without the account: 120 + 160 + 112 + 2 x 8 = 408 in full forms.
        resizeTo(container, 408);
        expect(layout(container).length).toBe(1);
        expect(real(container, ".agent-composer-strip-compact-btn").textContent).toBe("Compact");
        expect(real(container, ".agent-composer-strip-log-btn").textContent).toBe("Shell");
    });

    it("leaves the forms alone until the probes report a width", () => {
        const { container } = renderStrip(authed);
        resizeTo(container, 200);
        expect(real(container, ".agent-composer-strip-auth").textContent).toContain("alice");
        expect(real(container, ".agent-composer-strip-log-btn").textContent).toBe("Shell");
    });

    it("measures a probe again when its own width changes at a constant strip width (the readout gaining digits)", () => {
        readout.on = true;
        widths.stats = 80;
        Object.assign(probeWidths, PROBES, { "stats:full": 80, "stats:compact": 56 });
        const { container } = renderStrip({ authEmail: "alice@example.com" });
        resizeTo(container, 408);
        expect(real(container, ".block-stats-badge").dataset.form).toBe("full");
        // The live figures grow: the probe is 200 px now, the strip is the same width.
        probeWidths["stats:full"] = 200;
        for (const o of observers) {
            if ([...o.targets].some((t) => (t as HTMLElement).dataset?.probe === "stats:full")) o.cb([]);
        }
        expect(real(container, ".block-stats-badge").dataset.form).toBe("compact");
    });

    it("keeps the probes out of the rows, hidden from assistive tech and inert", () => {
        withProbes();
        const { container } = renderStrip(authed);
        resizeTo(container, 400);
        const probes = container.querySelector(".agent-composer-strip-probes")!;
        expect(probes.getAttribute("aria-hidden")).toBe("true");
        expect(probes.hasAttribute("inert")).toBe(true);
        expect(probes.closest(".agent-composer-strip-rows")).toBeNull();
        // Three slots in two forms each, and the account's four middle forms (composer-forms.ts ACCOUNT_BUDGETS).
        expect(probes.querySelectorAll("[data-probe]").length).toBe(6 + 4);
    });
});

describe("AgentComposerStrip — the CPU and memory readout", () => {
    const keysOf = (container: HTMLElement) => layout(container).flatMap((r) => [...r.left, ...r.right]);

    it("is no slot until the pane has its first figures", () => {
        const { container } = renderStrip();
        resizeTo(container, 1000);
        expect(keysOf(container)).not.toContain("stats");
        expect(container.querySelector(".block-stats-badge")).toBeNull();
    });

    it("is a slot of the strip beside the context reading, in the full form when the row has room", () => {
        readout.on = true;
        widths.stats = 80;
        const { container } = renderStrip();
        resizeTo(container, 1000);
        const keys = keysOf(container);
        // On the right, passive slots sit on the inner side: the readout, then the context reading, then Shell.
        expect(keys.slice(keys.indexOf("stats"))).toEqual(["stats", "ctx", "hostShell"]);
        expect(real(container, ".block-stats-badge").dataset.form).toBe("full");
    });

    it("counts toward the row: one slot more is 88 px and a gap wider", () => {
        readout.on = true;
        widths.stats = 80;
        const { container } = renderStrip();
        expect(narrowestOneRow(container)).toBe(520 + 80 + 8);
    });

    it("gives way last: the account, the context reading and Shell shrink first, then the readout to its chip", () => {
        readout.on = true;
        widths.stats = 80;
        Object.assign(probeWidths, PROBES, { "stats:full": 80, "stats:compact": 56 });
        const { container } = renderStrip({ authEmail: "alice@example.com" });
        const chip = () => real(container, ".block-stats-badge");

        // 120 + 32 + 112 + 80 + 32 + 4 x 8 = 408 with the three others compact: the readout is still full.
        resizeTo(container, 408);
        expect(layout(container).length).toBe(1);
        expect(chip().dataset.form).toBe("full");
        expect(real(container, ".agent-composer-strip-log-btn").getAttribute("aria-label")).toBe("Shell");

        // Its chip is 24 px smaller: one row down to 384, never hidden.
        resizeTo(container, 384);
        expect(layout(container).length).toBe(1);
        expect(chip().dataset.form).toBe("compact");

        resizeTo(container, 200);
        expect(keysOf(container)).toContain("stats");
    });
});

describe("AgentComposerStrip — a ~420 px pane (the operator's screenshot, #30203)", () => {
    it("is two lines, the readout beside the context reading, the anchors on the last", () => {
        readout.on = true;
        // The parts as a 420 px pane measured them: all compact they add up to ~450 px, so one line is out of reach.
        Object.assign(widths, { runtime: 172, auth: 160, ctxReading: 90, compact: 70, shell: 25, badge: 42, stats: 90 });
        Object.assign(probeWidths, {
            "auth:full": 160,
            "auth:compact": 24,
            "ctx:full": 168,
            "ctx:compact": 72,
            "hostShell:full": 140,
            "hostShell:compact": 72,
            "stats:full": 90,
            "stats:compact": 56,
        });
        const { container } = renderStrip({ authEmail: "alice@example.com" });
        resizeTo(container, 404);
        const rows = layout(container);
        expect(rows).toHaveLength(2);
        const first = [...rows[0].left, ...rows[0].right];
        expect(first).toEqual(expect.arrayContaining(["stats", "ctx"]));
        expect(Math.abs(first.indexOf("stats") - first.indexOf("ctx"))).toBe(1);
        const last = [...rows[1].left, ...rows[1].right];
        expect(last).toEqual(expect.arrayContaining(["runtime", "hostShell"]));
        // Shell is the outermost slot on the right, runtime on the left, whatever else shares the line.
        expect(rows[1].right.at(-1)).toBe("hostShell");
        expect(rows[1].left[0]).toBe("runtime");
    });
});

describe("AgentComposerStrip — the view chip", () => {
    const chipProps = { viewChip: "Replies · this pane", onOpenView: () => {} };

    it("shrinks to an eye, named by its text, after Shell and before the readout gives way", () => {
        widths.view = 150;
        Object.assign(probeWidths, PROBES, { "view:full": 150, "view:compact": 24 });
        const { container } = renderStrip({ authEmail: "alice@example.com", ...chipProps });
        const chip = () => real(container, ".agent-composer-strip-view");
        resizeTo(container, 1000);
        expect(chip().textContent).toBe("Replies · this pane");
        // 120 + 32 + 112 + 32 + 24 + 4 x 8 = 352 with every compact form; 480 with the chip still in words.
        resizeTo(container, 400);
        expect(layout(container).length).toBe(1);
        expect(chip().textContent).toBe("");
        expect(chip().getAttribute("aria-label")).toBe("Replies · this pane");
        expect(chip().getAttribute("title")).toContain("Replies · this pane");
    });

    it("lays the strip out again when the chip appears or goes while the pane keeps its width", () => {
        widths.view = 150;
        const [chip, setChip] = createSignal<string | null>(null);
        const utils = render(() => <AgentComposerStrip {...baseProps} viewChip={chip()} onOpenView={() => {}} />);
        const strip = utils.container.querySelector(".agent-composer-strip") as HTMLElement;
        Object.defineProperty(strip, "clientWidth", { configurable: true, value: 100 });
        resizeTo(utils.container, 400);
        const before = layout(utils.container).length;
        expect(before).toBeGreaterThan(1);
        setChip("Replies · this pane");
        expect(layout(utils.container).flatMap((r) => [...r.left, ...r.right])).toContain("view");
        expect(layout(utils.container).length).toBeGreaterThan(1);
        setChip(null);
        expect(layout(utils.container).flatMap((r) => [...r.left, ...r.right])).not.toContain("view");
        expect(layout(utils.container).length).toBe(before);
    });
});
