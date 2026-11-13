// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The hover tooltip (`Tooltip`, which is `TooltipInner` in tooltip.tsx), pinned as a whole ahead of splitting it into
 * a state file and a thin component: each scenario mounts the real component over the real timing module, with
 * stand-ins for the anchored popover (which prints what it is given and lets the scenario say when it has been
 * placed) and the primary-button state (which the scenario sets), on a fake clock, and logs the page as an indented
 * tree and, in order, every call made to the timing module (a claim, a release, a forget, a warm check and its
 * answer) and every popover mounted or removed. The logs are snapshots recorded on the unsplit component; the split
 * must leave them byte for byte.
 * (tooltip.test.tsx asserts what the person sees; this pins the markup, the timing and the calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, onCleanup, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
    const T: string[] = [];
    const log = (line: string): void => {
        T.push(line);
    };
    const state = {
        names: new WeakMap<object, string>(),
        count: 0,
        shown: [] as Array<() => void>,
        primaryDown: false,
        releaseListeners: [] as Array<() => void>,
        hoverMatches: false,
    };
    const nameOf = (tip: object): string => {
        let n = state.names.get(tip);
        if (!n) {
            n = `tip${++state.count}`;
            state.names.set(tip, n);
        }
        return n;
    };
    return { T, log, state, nameOf };
});
const { T, log } = h;

vi.mock("./tooltip-timing", async (orig) => {
    const actual = await orig<typeof import("./tooltip-timing")>();
    return {
        ...actual,
        claimOpen: (tip: any, register?: boolean) => {
            log(`claimOpen ${h.nameOf(tip)} register=${register}`);
            return actual.claimOpen(tip, register);
        },
        releaseOpen: (tip: any) => {
            log(`releaseOpen ${h.nameOf(tip)}`);
            return actual.releaseOpen(tip);
        },
        forgetOpen: (tip: any) => {
            log(`forgetOpen ${h.nameOf(tip)}`);
            return actual.forgetOpen(tip);
        },
        isWarm: (tip?: any) => {
            const warm = actual.isWarm(tip);
            log(`isWarm ${tip ? h.nameOf(tip) : "-"} -> ${warm}`);
            return warm;
        },
    };
});
vi.mock("@/app/util/pointer-drag-state", () => ({
    isPrimaryButtonDown: () => h.state.primaryDown,
    onPrimaryButtonRelease: (cb: () => void) => {
        h.state.releaseListeners.push(cb);
        return () => {
            h.state.releaseListeners = h.state.releaseListeners.filter((c) => c !== cb);
        };
    },
}));
vi.mock("@/app/element/anchored-popover", () => ({
    AnchoredPopover: (props: any) => {
        const anchor = props.anchor as HTMLElement | null;
        log(
            `popover mounted placement=${props.placement} gutter=${props.gutter} hideUntilPlaced=${props.hideUntilPlaced} shellClass=${JSON.stringify(props.shellClass)} anchor=${anchor ? `${anchor.tagName.toLowerCase()}.${anchor.className}` : "none"}`,
        );
        h.state.shown.push(() => props.onShown?.());
        onCleanup(() => log("popover removed"));
        return (
            <div data-popover="" class={props.class} style={props.style}>
                {props.children}
            </div>
        );
    },
}));

import { Tooltip } from "./tooltip";
import { resetTooltipTiming } from "./tooltip-timing";

const SNAP = (name: string) => `./__snapshots__/tooltip.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|id)$/;

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

type Opts = Partial<{
    placement: "top" | "bottom" | "left" | "right";
    forceOpen: boolean;
    disable: boolean;
    delayMs: number;
    immediate: boolean;
    passive: boolean;
    divClassName: string;
    divStyle: Record<string, string>;
    content: JSX.Element;
}>;
interface Mounted {
    name: string;
    anchor: () => HTMLElement;
    set: (patch: Opts) => void;
    unmount: () => void;
}

const mounted: Mounted[] = [];
/** Mounts a tooltip over an anchor whose text is `name`, with its options kept in a signal. */
const mount = (name: string, initial: Opts = {}): Mounted => {
    log(`-- mount ${name}`);
    const [opts, setOpts] = createSignal<Opts>(initial);
    const r = render(() => (
        <Tooltip
            content={opts().content ?? <span>{name} content</span>}
            placement={opts().placement}
            forceOpen={opts().forceOpen}
            disable={opts().disable}
            delayMs={opts().delayMs}
            immediate={opts().immediate}
            passive={opts().passive}
            divClassName={opts().divClassName}
            divStyle={opts().divStyle as any}
            divOnClick={() => log(`click on ${name}`)}
        >
            <span>{name}</span>
        </Tooltip>
    ));
    const m: Mounted = {
        name,
        anchor: () => r.container.querySelector("div")!,
        set: (patch) => {
            log(`-- ${name}: set ${JSON.stringify(patch, (_k, v) => (typeof v === "object" && v && "t" in v ? "<node>" : v))}`);
            setOpts({ ...opts(), ...patch });
        },
        unmount: () => {
            log(`-- unmount ${name}`);
            r.unmount();
        },
    };
    mounted.push(m);
    return m;
};
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(document.body).join("\n"));
};
const settle = async (ms = 0) => {
    await vi.advanceTimersByTimeAsync(ms);
    for (let i = 0; i < 6; i++) await Promise.resolve();
};
const hover = async (m: Mounted, ms = 0) => {
    log(`-- hover ${m.name}`);
    fireEvent.mouseEnter(m.anchor());
    await settle(ms);
};
const leave = async (m: Mounted, ms = 0) => {
    log(`-- leave ${m.name}`);
    fireEvent.mouseLeave(m.anchor());
    await settle(ms);
};
/** The latest popover says it has been placed. */
const place = async () => {
    log("-- the popover is placed");
    h.state.shown[h.state.shown.length - 1]?.();
    await settle();
};
const wait = async (ms: number) => {
    log(`-- ${ms} ms pass`);
    await settle(ms);
};

beforeEach(() => {
    T.length = 0;
    h.state.count = 0;
    h.state.names = new WeakMap();
    h.state.shown = [];
    h.state.primaryDown = false;
    h.state.releaseListeners = [];
    mounted.length = 0;
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date", "performance"] });
    resetTooltipTiming();
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
});

describe("Tooltip, pinned as a whole", () => {
    it("shows after a short rest, lingers, and is quick to show again along a row", async () => {
        const a = mount("a");
        look("closed");
        await hover(a);
        look("hovering, 0 ms: open, not yet visible");
        await wait(39);
        look("39 ms");
        await wait(1);
        look("40 ms: visible, not placed");
        await place();
        look("placed");
        await leave(a);
        look("left");
        await wait(99);
        look("99 ms after leaving");
        await wait(1);
        look("100 ms after leaving: closed");
        await hover(a);
        look("hovered again at once: warm, so no rest");
        await place();
        await leave(a, 100);
        await wait(500);
        await hover(a);
        look("hovered after the warm time: rests again");
        await wait(40);
        look("visible again");
        await snapshot("default");
    });

    it("takes its options: a delay, immediate, passive, a placement, and the anchor's class, style and click", async () => {
        const a = mount("a", { delayMs: 200, placement: "bottom", divClassName: "anchor-class", divStyle: { color: "red" } });
        await hover(a);
        await wait(199);
        look("delay 200: 199 ms");
        await wait(1);
        look("delay 200: 200 ms");
        await place();
        fireEvent.click(a.anchor());
        await leave(a);
        await wait(199);
        look("delay 200: 199 ms after leaving");
        await wait(1);
        look("delay 200: closed");

        const b = mount("b", { immediate: true, placement: "left" });
        await hover(b);
        look("immediate: open and visible, not placed");
        await place();
        look("immediate: placed");
        await leave(b);
        look("immediate: gone at once");

        const c = mount("c", { passive: true, placement: "right" });
        await hover(c, 50);
        await place();
        look("passive");
        await leave(c, 100);

        const d = mount("d", { immediate: true, delayMs: 500 });
        await hover(d);
        await place();
        look("immediate overrides the delay");
        await leave(d);
        await snapshot("options");
    });

    it("can be held open and switched off", async () => {
        const a = mount("a", { forceOpen: true });
        look("forced open at mount");
        await place();
        look("forced open, placed");
        await hover(a);
        await leave(a);
        look("pointer came and went while held");
        a.set({ forceOpen: false });
        await settle();
        look("hold ended, pointer away: lingers");
        await wait(100);
        look("closed");

        const b = mount("b");
        await hover(b, 40);
        await place();
        look("open");
        b.set({ disable: true });
        await settle();
        look("disabled while open: closed at once");
        b.set({ disable: false });
        await settle(60);
        look("enabled with the pointer still there: opens");
        await place();
        b.set({ forceOpen: true, disable: true });
        await settle();
        look("disabled and forced: disable wins");
        b.set({ disable: false });
        await settle();
        look("held open again");
        b.set({ forceOpen: false });
        await settle();
        look("hold ended with the pointer still over it");
        await leave(b, 100);

        const c = mount("c", { disable: true });
        await hover(c, 100);
        look("disabled from the start: never opens");
        await leave(c, 200);
        await snapshot("hold");
    });

    it("ignores a hover that starts mid-drag, and reopens when the button is released over it", async () => {
        const a = mount("a");
        h.state.primaryDown = true;
        await hover(a, 100);
        look("primary button down: hover ignored");
        const matches = vi.spyOn(Element.prototype, "matches").mockImplementation(function (this: Element, sel: string) {
            log(`matches ${sel} -> ${h.state.hoverMatches}`);
            return h.state.hoverMatches;
        });
        h.state.primaryDown = false;
        h.state.hoverMatches = false;
        log("-- the button is released, the pointer is not over the anchor");
        h.state.releaseListeners.forEach((cb) => cb());
        await settle(100);
        look("released elsewhere: still closed");
        h.state.hoverMatches = true;
        log("-- the button is released over the anchor");
        h.state.releaseListeners.forEach((cb) => cb());
        await settle(100);
        look("released over it: opens");
        await place();
        h.state.primaryDown = true;
        await leave(a, 100);
        look("a leave mid-drag still closes it");
        matches.mockRestore();

        const b = mount("b", { forceOpen: true });
        h.state.primaryDown = false;
        log("-- the button is released while held open");
        h.state.releaseListeners.forEach((cb) => cb());
        await settle();
        look("held open: release does nothing");
        await snapshot("drag");
    });

    it("closes the other tooltips when it opens, and lets go when it is removed", async () => {
        const a = mount("a");
        const b = mount("b");
        await hover(a, 40);
        await place();
        look("a open");
        await leave(a);
        await hover(b);
        look("b hovered while a lingers: a closed, b at once");
        await place();
        await leave(b, 100);
        await wait(500);

        const c = mount("c", { immediate: true });
        await hover(c);
        await place();
        look("an immediate one open");
        await hover(a);
        look("a opens: the immediate one closes");
        await place();
        await leave(a, 100);
        await leave(c);

        const d = mount("d");
        await hover(d, 40);
        await place();
        d.unmount();
        look("removed while open");
        await wait(500);
        await snapshot("several");
    });
});
