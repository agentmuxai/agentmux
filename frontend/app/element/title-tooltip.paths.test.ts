// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/**
 * What initTitleTooltips does around the edges that title-tooltip.test.ts leaves open: being started
 * twice, how the panel is placed and followed, the hand-over from one titled element to the next, a
 * pending close that the pointer coming back cancels, how soon the next tooltip shows after one
 * closed, and which events it listens for (and takes back on stop).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const stops: ReturnType<typeof vi.fn>[] = [];
const updates: (() => void)[] = [];
let nextPositions: { x: number; y: number }[] = [];
const pending: ((pos: { x: number; y: number }) => void)[] = [];

vi.mock("@floating-ui/dom", () => ({
    autoUpdate: vi.fn((_reference: unknown, _floating: unknown, update: () => void) => {
        const stopFn = vi.fn();
        stops.push(stopFn);
        updates.push(update);
        return stopFn;
    }),
    computePosition: vi.fn(
        () =>
            new Promise((resolve) => {
                const held = nextPositions.shift();
                if (held === undefined) pending.push(resolve);
                else resolve(held);
            })
    ),
    flip: vi.fn(() => ({ name: "flip" })),
    offset: vi.fn((n: number) => ({ name: "offset", n })),
    shift: vi.fn((o: unknown) => ({ name: "shift", o })),
}));

import { autoUpdate, computePosition } from "@floating-ui/dom";
import { initTitleTooltips } from "./title-tooltip";
import { claimOpen, HIDE_DELAY_MS, resetTooltipTiming, SHOW_DELAY_MS, WARM_MS } from "./tooltip-timing";

let stop: () => void;
beforeEach(() => {
    vi.useFakeTimers();
    stops.length = 0;
    updates.length = 0;
    pending.length = 0;
    // Every placement resolves at once unless a test holds it back.
    nextPositions = Array.from({ length: 50 }, () => ({ x: 7, y: 9 }));
    vi.mocked(autoUpdate).mockClear();
    vi.mocked(computePosition).mockClear();
    stop = initTitleTooltips(document);
    resetTooltipTiming();
});
afterEach(() => {
    stop();
    vi.useRealTimers();
    document.body.innerHTML = "";
});

const panel = () => document.body.querySelector<HTMLElement>("[data-pane-overlay]");
const body = () => panel()!.firstElementChild as HTMLElement;
const over = (el: Element) => el.dispatchEvent(new Event("pointerover", { bubbles: true }));
const settle = (ms: number) => vi.advanceTimersByTimeAsync(ms);

function add(html: string): HTMLElement {
    const host = document.body.appendChild(document.createElement("div"));
    host.innerHTML = html;
    return host.firstElementChild as HTMLElement;
}

/** Open the tooltip on `el` and let it be shown. */
async function open(el: Element) {
    over(el);
    await settle(SHOW_DELAY_MS + 10);
}

describe("starting twice", () => {
    it("returns the same stop and listens once", () => {
        expect(initTitleTooltips(document)).toBe(stop);
    });
});

describe("how the panel is placed", () => {
    it("is placed above the element in fixed pixels, with a gap, flipping and kept off the edges", async () => {
        const el = add('<button title="Close">x</button>');
        await open(el);

        const calls = vi.mocked(computePosition).mock.calls;
        expect(calls.length).toBeGreaterThan(0);
        const [reference, floating, options] = calls[0];
        expect(reference).toBe(el);
        expect(floating).toBe(panel());
        expect(options).toEqual({
            strategy: "fixed",
            placement: "top",
            middleware: [{ name: "offset", n: 10 }, { name: "flip" }, { name: "shift", o: { padding: 12 } }],
        });
    });

    it("follows the element while it is open: the position is worked out again when it moves", async () => {
        const el = add('<button title="Close">x</button>');
        await open(el);
        expect(vi.mocked(autoUpdate)).toHaveBeenCalledWith(el, panel(), expect.any(Function));

        const before = vi.mocked(computePosition).mock.calls.length;
        nextPositions = [{ x: 31, y: 41 }];
        updates[0]();
        await settle(0);

        expect(vi.mocked(computePosition).mock.calls.length).toBe(before + 1);
        expect(panel()!.style.left).toBe("31px");
        expect(panel()!.style.top).toBe("41px");
    });

    it("stops following when it closes, once", async () => {
        const el = add('<button title="Close">x</button>');
        await open(el);
        el.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true }));
        window.dispatchEvent(new Event("blur"));

        expect(stops).toHaveLength(1);
        expect(stops[0]).toHaveBeenCalledTimes(1);
    });
});

describe("moving from one titled element to the next", () => {
    it("gives the first one its title back and stops following it", async () => {
        const a = add('<button title="Alpha">a</button>');
        const b = add('<button title="Beta">b</button>');
        await open(a);
        expect(a.getAttribute("title")).toBe("");

        over(b);
        await settle(0);

        expect(a.getAttribute("title")).toBe("Alpha");
        expect(a.dataset.titleTip).toBeUndefined();
        expect(stops[0]).toHaveBeenCalledTimes(1);
        expect(panel()!.textContent).toBe("Beta");
    });

    it("keeps the new tooltip hidden until it is placed, not at the old one's position", async () => {
        const a = add('<button title="Alpha">a</button>');
        const b = add('<button title="Beta">b</button>');
        await open(a);
        expect(body().style.opacity).toBe("1");

        nextPositions = [];
        over(b);
        await settle(0);
        expect(body().style.opacity).toBe("0");

        pending.shift()!({ x: 3, y: 4 });
        await settle(0);
        expect(body().style.opacity).toBe("1");
        expect(panel()!.style.left).toBe("3px");
    });

    it("does not let a late placement for the first element move the second", async () => {
        const a = add('<button title="Alpha">a</button>');
        const b = add('<button title="Beta">b</button>');
        nextPositions = [];
        over(a);
        await settle(SHOW_DELAY_MS + 10);
        over(b);
        await settle(0);
        expect(pending).toHaveLength(2);

        pending[0]({ x: 1, y: 2 });
        await settle(0);
        expect(panel()!.style.left).not.toBe("1px");

        pending[1]({ x: 5, y: 6 });
        await settle(0);
        expect(panel()!.style.left).toBe("5px");
    });
});

describe("a close that is waiting out its grace", () => {
    it("is cancelled when the pointer reaches another titled element, and the new one still closes later", async () => {
        const a = add('<button title="Alpha">a</button>');
        const b = add('<button title="Beta">b</button>');
        const plain = add("<span>p</span>");
        const plain2 = add("<span>q</span>");
        await open(a);

        over(plain);
        await settle(30);
        over(plain2);
        await settle(30);
        over(b);
        await settle(HIDE_DELAY_MS * 3);
        expect(panel()!.textContent).toBe("Beta");

        over(plain);
        await settle(HIDE_DELAY_MS);
        expect(panel()).toBeNull();
    });

    it("is cancelled when the pointer comes back onto the element, and a later leave still closes", async () => {
        const a = add('<button title="Alpha">a</button>');
        const plain = add("<span>p</span>");
        await open(a);

        over(plain);
        await settle(HIDE_DELAY_MS / 2);
        over(a);
        await settle(HIDE_DELAY_MS * 3);
        expect(panel()!.textContent).toBe("Alpha");

        over(plain);
        await settle(HIDE_DELAY_MS);
        expect(panel()).toBeNull();
    });

    it("stays open when the pointer reaches an element with an empty title, and goes after the grace", async () => {
        const a = add('<button title="Alpha">a</button>');
        const empty = add('<button title="">e</button>');
        await open(a);

        over(empty);
        expect(panel()!.textContent).toBe("Alpha");
        await settle(HIDE_DELAY_MS - 1);
        expect(panel()).not.toBeNull();
        await settle(1);
        expect(panel()).toBeNull();
    });
});

describe("the next tooltip after one closed", () => {
    it("shows at once, not after the short rest", async () => {
        const a = add('<button title="Alpha">a</button>');
        const b = add('<button title="Beta">b</button>');
        await open(a);
        a.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true }));
        expect(panel()).toBeNull();

        over(b);
        await settle(1);

        expect(body().style.opacity).toBe("1");
    });

    it("keeps a panel it opens again hidden until the rest is over, once nothing is warm any more", async () => {
        const a = add('<button title="Alpha">a</button>');
        const b = add('<button title="Beta">b</button>');
        await open(a);
        expect(body().style.opacity).toBe("1");
        a.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true }));
        await settle(WARM_MS + 100);

        over(b);
        // Before anything is placed: the panel from the last time is not shown at the old position.
        expect(body().style.opacity).toBe("0");
        await settle(SHOW_DELAY_MS - 1);
        expect(body().style.opacity).toBe("0");
        await settle(1);
        expect(body().style.opacity).toBe("1");
    });

    it("waits out the short rest from a cold start", async () => {
        const a = add('<button title="Alpha">a</button>');
        over(a);
        await settle(SHOW_DELAY_MS - 1);
        expect(body().style.opacity).toBe("0");
        await settle(1);
        expect(body().style.opacity).toBe("1");
    });
});

describe("what makes it let go", () => {
    it("another tooltip opening closes it and gives the title back", async () => {
        const a = add('<button title="Alpha">a</button>');
        await open(a);
        expect(a.getAttribute("title")).toBe("");

        claimOpen({ close: vi.fn() });

        expect(panel()).toBeNull();
        expect(a.getAttribute("title")).toBe("Alpha");
    });

    it("a key press closes it and the title stays taken until the pointer leaves", async () => {
        const a = add('<button title="Alpha">a</button>');
        await open(a);

        a.dispatchEvent(new KeyboardEvent("keydown", { bubbles: false }));

        expect(panel()).toBeNull();
        expect(a.getAttribute("title")).toBe("");
    });

    it("a press closes it even when the event does not bubble", async () => {
        const a = add('<button title="Alpha">a</button>');
        await open(a);

        a.dispatchEvent(new Event("pointerdown", { bubbles: false }));

        expect(panel()).toBeNull();
        expect(a.getAttribute("title")).toBe("");
    });

    it("a wheel turn closes it even when the event does not bubble", async () => {
        const a = add('<button title="Alpha">a</button>');
        await open(a);

        a.dispatchEvent(new Event("wheel", { bubbles: false }));

        expect(panel()).toBeNull();
        expect(a.getAttribute("title")).toBe("");
    });

    it("the pointer leaving the page closes it after the grace", async () => {
        const a = add('<button title="Alpha">a</button>');
        await open(a);

        document.documentElement.dispatchEvent(new Event("pointerleave"));
        await settle(HIDE_DELAY_MS - 1);
        expect(panel()).not.toBeNull();
        await settle(1);
        expect(panel()).toBeNull();
        expect(a.getAttribute("title")).toBe("Alpha");
    });

    it("the window losing focus closes it at once and gives the title back, even after a click", async () => {
        const a = add('<button title="Alpha">a</button>');
        await open(a);
        a.dispatchEvent(new Event("pointerdown", { bubbles: true }));
        expect(a.getAttribute("title")).toBe("");

        window.dispatchEvent(new Event("blur"));

        expect(panel()).toBeNull();
        expect(a.getAttribute("title")).toBe("Alpha");
    });
});

describe("the listeners it adds", () => {
    type Call = { target: unknown; type: string; listener: unknown; capture: boolean };
    const capture = (options: unknown) =>
        typeof options === "boolean" ? options : !!(options as { capture?: boolean } | undefined)?.capture;
    let added: Call[];
    let removed: Call[];

    beforeEach(() => {
        stop();
        added = [];
        removed = [];
        const add = EventTarget.prototype.addEventListener;
        const remove = EventTarget.prototype.removeEventListener;
        vi.spyOn(EventTarget.prototype, "addEventListener").mockImplementation(function (
            this: EventTarget,
            type: string,
            listener: EventListenerOrEventListenerObject | null,
            options?: boolean | AddEventListenerOptions
        ) {
            added.push({ target: this, type, listener, capture: capture(options) });
            return add.call(this, type, listener, options);
        });
        vi.spyOn(EventTarget.prototype, "removeEventListener").mockImplementation(function (
            this: EventTarget,
            type: string,
            listener: EventListenerOrEventListenerObject | null,
            options?: boolean | EventListenerOptions
        ) {
            removed.push({ target: this, type, listener, capture: capture(options) });
            return remove.call(this, type, listener, options);
        });
        // jsdom's window has methods of its own, apart from the prototype's.
        const win = document.defaultView!;
        const winAdd = win.addEventListener.bind(win);
        const winRemove = win.removeEventListener.bind(win);
        vi.spyOn(win, "addEventListener").mockImplementation((type: string, listener: unknown, options?: unknown) => {
            added.push({ target: win, type, listener, capture: capture(options) });
            return winAdd(type, listener as EventListener, options as AddEventListenerOptions);
        });
        vi.spyOn(win, "removeEventListener").mockImplementation(
            (type: string, listener: unknown, options?: unknown) => {
                removed.push({ target: win, type, listener, capture: capture(options) });
                return winRemove(type, listener as EventListener, options as EventListenerOptions);
            }
        );
        stop = initTitleTooltips(document);
    });
    afterEach(() => vi.restoreAllMocks());

    it("listens on the document in the capture phase, on the page root for the pointer leaving, and on the window for blur", () => {
        const mine = added.filter(
            (c) => c.target === document || c.target === document.documentElement || c.target === document.defaultView
        );
        expect(
            mine.map((c) => [
                c.target === document ? "document" : c.target === document.defaultView ? "window" : "root",
                c.type,
                c.capture,
            ])
        ).toEqual([
            ["document", "pointerover", true],
            ["document", "pointerdown", true],
            ["document", "keydown", true],
            ["document", "wheel", true],
            ["root", "pointerleave", false],
            ["window", "blur", false],
        ]);
    });

    it("takes back each one it added, the same way, when stopped", () => {
        const mine = added.filter(
            (c) => c.target === document || c.target === document.documentElement || c.target === document.defaultView
        );
        stop();

        for (const c of mine) {
            expect(
                removed.some(
                    (r) =>
                        r.target === c.target &&
                        r.type === c.type &&
                        r.listener === c.listener &&
                        r.capture === c.capture
                ),
                `${c.type} removed`
            ).toBe(true);
        }
    });
});
