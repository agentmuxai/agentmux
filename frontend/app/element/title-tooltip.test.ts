// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@floating-ui/dom", () => ({
    autoUpdate: vi.fn(() => vi.fn()),
    computePosition: vi.fn(() => Promise.resolve({ x: 7, y: 9 })),
    flip: vi.fn(() => ({})),
    offset: vi.fn(() => ({})),
    shift: vi.fn(() => ({})),
}));

import { giveTitleBack, initTitleTooltips, takeTitle } from "./title-tooltip";
import { resetTooltipTiming } from "./tooltip-timing";

let stop: () => void;
beforeEach(() => {
    vi.useFakeTimers();
    stop = initTitleTooltips(document);
    // Whatever an earlier test left open or just closed no longer counts as warm.
    resetTooltipTiming();
});
afterEach(() => {
    stop();
    vi.useRealTimers();
    document.body.innerHTML = "";
});

const panel = () => document.body.querySelector<HTMLElement>("[data-pane-overlay]");
const over = (el: Element) => el.dispatchEvent(new Event("pointerover", { bubbles: true }));
const settle = (ms: number) => vi.advanceTimersByTimeAsync(ms);

function add(html: string): HTMLElement {
    const host = document.body.appendChild(document.createElement("div"));
    host.innerHTML = html;
    return host.firstElementChild as HTMLElement;
}

describe("takeTitle", () => {
    it("moves the title to data-title-tip and labels an element that has no text of its own", () => {
        const el = add('<button title="Close"><i></i></button>');
        expect(takeTitle(el)).toBe("Close");
        expect(el.hasAttribute("title")).toBe(false);
        expect(el.dataset.titleTip).toBe("Close");
        expect(el.getAttribute("aria-label")).toBe("Close");
    });

    it("describes an element that has text, and leaves an aria-label of its own alone", () => {
        const withText = add('<button title="Close this tab">Close</button>');
        takeTitle(withText);
        expect(withText.hasAttribute("aria-label")).toBe(false);
        expect(withText.getAttribute("aria-description")).toBe("Close this tab");

        const labelled = add('<button aria-label="Mine" title="Longer"><i></i></button>');
        takeTitle(labelled);
        expect(labelled.getAttribute("aria-label")).toBe("Mine");
    });

    it("replaces the label it set earlier when the title changes, and drops it for an empty title", () => {
        const el = add('<button title="One"><i></i></button>');
        takeTitle(el);
        el.setAttribute("title", "Two");
        expect(takeTitle(el)).toBe("Two");
        expect(el.getAttribute("aria-label")).toBe("Two");
        el.setAttribute("title", "");
        expect(takeTitle(el)).toBe("");
        expect(el.dataset.titleTip).toBeUndefined();
    });
});

describe("giveTitleBack", () => {
    it("puts the title back and removes the label it made", () => {
        const el = add('<button title="Close"><i></i></button>');
        takeTitle(el);
        giveTitleBack(el);
        expect(el.getAttribute("title")).toBe("Close");
        expect(el.hasAttribute("aria-label")).toBe(false);
        expect(el.hasAttribute("data-title-tip-aria")).toBe(false);
        expect(el.dataset.titleTip).toBeUndefined();
    });

    it("leaves a label of the element's own, and a title the application set meanwhile", () => {
        const el = add('<button aria-label="Mine" title="Close"><i></i></button>');
        takeTitle(el);
        el.setAttribute("title", "Newer");
        giveTitleBack(el);
        expect(el.getAttribute("aria-label")).toBe("Mine");
        expect(el.getAttribute("title")).toBe("Newer");
    });
});

describe("the delegated tooltip", () => {
    it("gives the title back when the pointer has left, so a title the application removes later shows nothing", async () => {
        const el = add('<button title="Cancel sign-in">x</button>');
        const plain = add('<span class="plain">y</span>');
        over(el);
        await settle(100);
        over(plain);
        await settle(150);
        expect(panel()).toBeNull();
        expect(el.getAttribute("title")).toBe("Cancel sign-in");
        // The application clears it (a reactive label that is no longer set).
        el.removeAttribute("title");
        over(el);
        await settle(150);
        expect(panel()).toBeNull();
    });

    it("keeps showing while the pointer moves inside an element whose title it took", async () => {
        const el = add('<button title="Save"><i class="icon"></i></button>');
        over(el);
        await settle(100);
        over(el.querySelector(".icon")!);
        await settle(150);
        expect(panel()!.textContent).toBe("Save");
    });

    it("shows the shared tooltip after the short rest, instead of the native one", async () => {
        const el = add('<button title="Save"><i></i></button>');
        over(el);
        expect(el.hasAttribute("title")).toBe(false);
        await settle(20);
        expect(panel()!.textContent).toBe("Save");
        expect(panel()!.style.opacity).toBe("0");
        await settle(60);
        expect(panel()!.style.opacity).toBe("1");
        expect(panel()!.style.left).toBe("7px");
        expect(panel()!.getAttribute("role")).toBe("tooltip");
    });

    it("uses the nearest titled ancestor when the pointer is over a child", async () => {
        const el = add('<button title="Save"><i class="child"></i></button>');
        over(el.querySelector(".child")!);
        await settle(100);
        expect(panel()!.textContent).toBe("Save");
    });

    it("goes after a short grace when the pointer moves to something without a title", async () => {
        const el = add('<div><button title="Save">x</button><span class="plain">y</span></div>').querySelector("button")!;
        over(el);
        await settle(100);
        over(document.querySelector(".plain")!);
        await settle(50);
        expect(panel()).not.toBeNull();
        await settle(100);
        expect(panel()).toBeNull();
    });

    it("moves to the next titled element at once, from a cold start", async () => {
        const host = add('<div><button title="One">1</button><button title="Two">2</button></div>');
        const [one, two] = [...host.querySelectorAll("button")];
        over(one);
        await settle(20);
        // The first one is still within its 40 ms rest: a cold start waits.
        expect(panel()!.style.opacity).toBe("0");
        await settle(100);
        over(two);
        await settle(20);
        expect(document.body.querySelectorAll("[data-pane-overlay]")).toHaveLength(1);
        expect(panel()!.textContent).toBe("Two");
        expect(panel()!.style.opacity).toBe("1");
    });

    it("closes when something is pressed", async () => {
        const el = add('<button title="Save">x</button>');
        over(el);
        await settle(100);
        el.dispatchEvent(new Event("pointerdown", { bubbles: true }));
        expect(panel()).toBeNull();
    });

    it("leaves option, iframe and data-native-title elements to the browser", async () => {
        const host = add(
            '<div><select><option title="Native">a</option></select><iframe title="Frame"></iframe><span data-native-title title="Mine">z</span></div>'
        );
        for (const el of host.querySelectorAll("option, iframe, span")) {
            over(el);
            await settle(100);
            expect(el.getAttribute("title")).not.toBeNull();
            expect(panel()).toBeNull();
        }
    });

    it("leaves an element that has its own data-tip tooltip alone", async () => {
        const el = add('<span data-tip="Mine">x</span>');
        over(el);
        await settle(100);
        expect(panel()).toBeNull();
    });

    it("shows nothing for an empty title", async () => {
        const el = add('<button title="">x</button>');
        over(el);
        await settle(100);
        expect(panel()).toBeNull();
    });

    it("does not start while the primary button is held, but takes the title later", async () => {
        const pointer = await import("@/app/util/pointer-drag-state");
        const spy = vi.spyOn(pointer, "isPrimaryButtonDown").mockReturnValue(true);
        const el = add('<button title="Save">x</button>');
        over(el);
        await settle(100);
        expect(panel()).toBeNull();
        spy.mockReturnValue(false);
        over(el);
        await settle(100);
        expect(panel()!.textContent).toBe("Save");
        spy.mockRestore();
    });

    it("stops listening, and removes its panel, when stopped", async () => {
        const el = add('<button title="Save">x</button>');
        over(el);
        await settle(100);
        stop();
        expect(panel()).toBeNull();
        over(el);
        await settle(100);
        expect(panel()).toBeNull();
        stop = initTitleTooltips(document);
    });
});
