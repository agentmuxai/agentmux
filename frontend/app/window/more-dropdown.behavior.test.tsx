// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MoreDropdown (the widget bar's overflow menu), pinned as a whole ahead of splitting it into smaller functions: each
 * scenario mounts the real dropdown with stand-ins for the anchored popover, the row flyout and the widget actions,
 * wraps the real hover controllers and peer registry so every call they get is logged, then hovers, clicks and
 * right-clicks its rows and logs the dropdown as an indented tree and, in order, every call it makes. The logs are
 * snapshots recorded on the unsplit component; the split must leave them byte for byte. (more-dropdown.test.tsx and
 * more-dropdown.crossing.test.tsx keep the pointer-crossing checks they were written for.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, onCleanup, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({ pop: null as any, flyouts: [] as any[], hoverIds: 0 }));

vi.mock("@/app/util/submenu-hover", async () => {
    const actual: any = await vi.importActual("@/app/util/submenu-hover");
    const wrap = <T extends object>(label: string, target: T, skip: string[] = []): T =>
        new Proxy(target, {
            get(t: any, k) {
                const v = t[k];
                if (typeof v !== "function" || skip.includes(String(k))) return v;
                return (...args: any[]) => (
                    log(`${label}.${String(k)}${args.length && typeof args[0] === "string" ? " " + args[0] : ""}`),
                    v(...args)
                );
            },
        });
    return {
        ...actual,
        createPeerRegistry: () => wrap("peers", actual.createPeerRegistry()),
        createSubmenuHover: (opts: any) => {
            const id = ++h.hoverIds;
            log(`createSubmenuHover #${id} closeOnLeave=${opts.closeOnLeave}`);
            return wrap(
                `hover#${id}`,
                actual.createSubmenuHover({
                    ...opts,
                    onOpen: () => (log(`hover#${id} opens its submenu`), opts.onOpen()),
                    onClose: () => (log(`hover#${id} closes its submenu`), opts.onClose()),
                }),
                ["closeForPeer"]
            );
        },
    };
});
vi.mock("@/app/element/anchored-popover", () => ({
    AnchoredPopover: (p: any) => {
        h.pop = p;
        return (
            <div
                data-stand="AnchoredPopover"
                data-placement={p.placement}
                data-gutter={p.gutter}
                data-hide-until-placed={String(!!p.hideUntilPlaced)}
                data-shell={p.shellClass}
                class={p.class}
                ref={(el) => p.ref?.(el)}
            >
                {p.children}
            </div>
        );
    },
}));
vi.mock("./pinned-widget-flyout", () => ({
    PinnedWidgetFlyout: (p: any) => {
        h.flyouts.push(p);
        log(
            `PinnedWidgetFlyout mounted: ${p.widget.label} placement=${p.placement} gutter=${p.gutter} anchor=${
                (p.anchor() as HTMLElement | null)?.dataset?.rowKey ?? String(p.anchor())
            }`
        );
        onCleanup(() => (log(`PinnedWidgetFlyout ${p.widget.label} cleanup`), p.setSubmenuEl?.(null)));
        return (
            <div data-stand="PinnedWidgetFlyout" data-label={p.widget.label} ref={(el) => p.setSubmenuEl?.(el)} />
        );
    },
}));
vi.mock("./action-widgets-config", () => ({
    openWidget: (w: any, o: any) => log(`openWidget ${w.label} ${JSON.stringify(o)}`),
    widgetIconStyle: (w: any) => ({ "--widget-icon-color": `c-${w.label}` }),
}));

import { MoreDropdown } from "./more-dropdown";

const SNAP = (name: string) => `./__snapshots__/more-dropdown.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|value|placeholder)$/;

/** The dropdown as an indented tree: tags, classes, the attributes that carry state, and text. */
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

const widget = (label: string, extra: object = {}) =>
    ({ label, icon: "window-maximize", blockdef: { meta: { view: "term" } }, ...extra }) as unknown as WidgetConfigType;
const entry = (key: string, w: WidgetConfigType) => ({ key, widget: w });

const rect = (left: number, top: number, right: number, bottom: number) =>
    ({ x: left, y: top, left, top, right, bottom, width: right - left, height: bottom - top }) as DOMRect;

let host: HTMLElement;
let unmount: () => void;
let setWidgets: (w: { key: string; widget: WidgetConfigType }[]) => void;
let setItemMenuOpen: (open: boolean) => void;
const regions: ({ getBoundingClientRect(): DOMRect } | null)[] = [];
const anchorEl = () => host.querySelector<HTMLElement>("[data-anchor]")!;

const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host.querySelector("[data-stand='AnchoredPopover']")!).join("\n") || "-");
    log(`flyouts open: ${[...host.querySelectorAll("[data-stand='PinnedWidgetFlyout']")].map((f) => (f as HTMLElement).dataset.label).join(", ") || "none"}`);
};
const rows = () => [...host.querySelectorAll<HTMLElement>(".action-widget-more-item")];
const row = (label: string) => rows().find((r) => r.textContent?.includes(label))!;
const aimed = (label: string) => {
    const r = regions.filter(Boolean).at(-1)?.getBoundingClientRect();
    log(`${label}: close intent aims at ${r ? [r.left, r.top, r.right, r.bottom].join(",") : "nothing"}`);
};

function mount(initial: { key: string; widget: WidgetConfigType }[], withHoverPlumbing = true) {
    const [widgets, setW] = createSignal(initial);
    const [itemMenu, setIM] = createSignal(false);
    setWidgets = setW;
    setItemMenuOpen = setIM;
    h.hoverIds = 0;
    const view = render(() => (
        <div>
            <span data-anchor />
            <MoreDropdown
                widgets={widgets}
                onClose={() => log("onClose")}
                onItemContextMenu={(pos, key) => log(`onItemContextMenu ${JSON.stringify(pos)} ${key}`)}
                anchor={() => (log("anchor read"), document.body)}
                settings={() => ({})}
                wmap={() => ({})}
                ref={(el) => log(`ref ${JSON.stringify(el.className)}`)}
                onSubmenuEnter={withHoverPlumbing ? () => log("onSubmenuEnter") : undefined}
                onSubmenuLeave={withHoverPlumbing ? (e) => log(`onSubmenuLeave ${e.clientX},${e.clientY}`) : undefined}
                setSubmenuEl={(el) => (regions.push(el), log(`setSubmenuEl ${el ? "region" : "null"}`))}
                isItemMenuOpen={withHoverPlumbing ? itemMenu : undefined}
            />
        </div>
    ));
    host = view.container;
    unmount = view.unmount;
    rows().forEach((r, i) => (r.dataset.rowKey = `row-${i}`));
}

const parentA = widget("Messengers", { children: ["discord"] });
const parentB = widget("Tools", { children: ["calc"] });
const plain = widget("Terminal");
const plainTwo = widget("Notes", { icon: "note-sticky" });

beforeEach(() => {
    T.length = 0;
    h.flyouts.length = 0;
    regions.length = 0;
    vi.useFakeTimers();
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("MoreDropdown, pinned", () => {
    it("places the dropdown under the More button, hands out its element, and closes only on Escape", () => {
        mount([entry("a", plain)]);
        log(
            `popover: placement=${h.pop.placement} gutter=${h.pop.gutter} hideUntilPlaced=${h.pop.hideUntilPlaced} shell=${h.pop.shellClass} class=${h.pop.class}`
        );
        look("one plain row");
        h.pop.onDismiss("escape");
        h.pop.onDismiss("outside");
        h.pop.onDismiss("scroll");
        log("(only Escape closed it)");
        aimed("the dropdown");
        unmount();
        log("unmounted");
        return snapshot("shell");
    });

    it("opens a plain row's widget on click (a new pane on shift-click) and closes, and right-click hands the position and key to the bar without closing", () => {
        mount([entry("a", plain), entry("b", plainTwo)]);
        fireEvent.mouseEnter(row("Terminal"));
        fireEvent.click(row("Terminal"));
        fireEvent.click(row("Notes"), { shiftKey: true });
        const ev = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 31, clientY: 47 });
        const bubbled = vi.fn();
        host.addEventListener("contextmenu", bubbled);
        row("Notes").dispatchEvent(ev);
        log(`context menu: prevented=${ev.defaultPrevented} reachedParent=${bubbled.mock.calls.length > 0}`);
        look("plain rows");
        return snapshot("plain-rows");
    });

    it("tells the More button when the pointer enters or leaves the dropdown (not while a row's item menu is open), including when it has no hover plumbing", () => {
        mount([entry("a", plain)]);
        const body = host.querySelector("[data-stand='AnchoredPopover']")!;
        body.dispatchEvent(new MouseEvent("mouseenter"));
        body.dispatchEvent(new MouseEvent("mouseleave", { clientX: 5, clientY: 6 }));
        setItemMenuOpen(true);
        log("item menu open");
        body.dispatchEvent(new MouseEvent("mouseleave", { clientX: 7, clientY: 8 }));
        setItemMenuOpen(false);
        body.dispatchEvent(new MouseEvent("mouseleave", { clientX: 9, clientY: 10 }));
        cleanup();
        T.push("-- without plumbing");
        mount([entry("a", plain)], false);
        const body2 = host.querySelector("[data-stand='AnchoredPopover']")!;
        body2.dispatchEvent(new MouseEvent("mouseenter"));
        body2.dispatchEvent(new MouseEvent("mouseleave", { clientX: 1, clientY: 2 }));
        log("(nothing to tell)");
        return snapshot("plumbing");
    });

    it("opens a parent row's flyout after the hover delay beside the row, keeps it open when the pointer leaves, and closes it when another row is entered", () => {
        mount([entry("m", parentA), entry("a", plain)]);
        look("parent row closed");
        fireEvent.mouseEnter(row("Messengers"));
        log("pointer on Messengers");
        vi.advanceTimersByTime(100);
        look("after the hover delay");
        fireEvent.mouseLeave(row("Messengers"), { clientX: 50, clientY: 10 });
        vi.advanceTimersByTime(1000);
        look("pointer left the row: flyout stays");
        fireEvent.mouseEnter(row("Terminal"), { clientX: 10, clientY: 90 });
        vi.advanceTimersByTime(1000);
        look("another row entered: flyout closed");
        return snapshot("parent-open-close");
    });

    it("hands the dropdown's hover intent to the flyout: entering it and leaving it reach the More button, a leave is ignored while an item menu is open, and the close intent aims at the flyout", () => {
        mount([entry("m", parentA)]);
        host.querySelector<HTMLElement>("[data-stand='AnchoredPopover']")!.getBoundingClientRect = () => rect(100, 0, 200, 80);
        fireEvent.mouseEnter(row("Messengers"));
        vi.advanceTimersByTime(100);
        const flyout = host.querySelector<HTMLElement>("[data-stand='PinnedWidgetFlyout']")!;
        flyout.getBoundingClientRect = () => rect(208, 10, 320, 60);
        aimed("flyout open");
        const f = h.flyouts[0];
        f.onSubmenuEnter();
        f.onSubmenuLeave({ clientX: 330, clientY: 20 });
        setItemMenuOpen(true);
        log("item menu open");
        f.onSubmenuLeave({ clientX: 331, clientY: 21 });
        fireEvent.mouseLeave(row("Messengers"), { clientX: 150, clientY: 10 });
        setItemMenuOpen(false);
        fireEvent.mouseLeave(row("Messengers"), { clientX: 151, clientY: 11 });
        fireEvent.mouseEnter(row("Messengers"));
        f.onSubmenuEnter();
        vi.advanceTimersByTime(1000);
        look("after leave/enter on the row");
        return snapshot("parent-flyout-hover");
    });

    it("closes the flyout and the dropdown when a flyout item is chosen, and passes the flyout's item menu requests on", () => {
        mount([entry("m", parentA)]);
        fireEvent.mouseEnter(row("Messengers"));
        vi.advanceTimersByTime(100);
        const f = h.flyouts[0];
        log(`flyout props: ${Object.keys(f).sort().join(",")}`);
        f.onItemContextMenu({ x: 1, y: 2 }, "discord");
        f.onClose();
        vi.advanceTimersByTime(1000);
        look("flyout item chosen");
        const evt = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 9, clientY: 8 });
        row("Messengers").dispatchEvent(evt);
        log(`parent row context menu prevented=${evt.defaultPrevented}`);
        fireEvent.mouseEnter(row("Messengers"));
        vi.advanceTimersByTime(100);
        look("hovering again opens it again");
        return snapshot("parent-choose");
    });

    it("lets a second parent row take over: entering it closes the first row's flyout and opens its own once the first is gone, only if the pointer is still on the row", () => {
        mount([entry("m", parentA), entry("t", parentB)]);
        fireEvent.mouseEnter(row("Messengers"));
        vi.advanceTimersByTime(100);
        look("Messengers open");
        fireEvent.mouseLeave(row("Messengers"), { clientX: 10, clientY: 10 });
        fireEvent.mouseEnter(row("Tools"), { clientX: 10, clientY: 40 });
        vi.advanceTimersByTime(1000);
        look("Tools entered");
        fireEvent.mouseEnter(row("Messengers"), { clientX: 10, clientY: 10 });
        fireEvent.mouseLeave(row("Messengers"), { clientX: 10, clientY: 30 });
        vi.advanceTimersByTime(1000);
        look("left Messengers before it opened: it stays shut, Tools closed");
        return snapshot("parent-takeover");
    });

    it("keeps each parent row's hover controller and an open flyout when the widget list is rebuilt with fresh objects, adds one for a new parent row, and disposes them on unmount", () => {
        mount([entry("m", parentA), entry("a", plain)]);
        fireEvent.mouseEnter(row("Messengers"));
        vi.advanceTimersByTime(100);
        log("-- rebuild with equal-looking fresh objects");
        setWidgets([entry("m", widget("Messengers", { children: ["discord"] })), entry("a", widget("Terminal"))]);
        look("after rebuild: flyout state kept");
        log("-- a new parent row appears");
        setWidgets([entry("m", widget("Messengers", { children: ["discord"] })), entry("t", parentB), entry("a", widget("Terminal"))]);
        look("after a second parent row appears");
        log("-- the first parent row goes away and comes back");
        setWidgets([entry("t", parentB)]);
        setWidgets([entry("m", widget("Messengers", { children: ["discord"] })), entry("t", parentB)]);
        look("Messengers back");
        unmount();
        log("unmounted");
        return snapshot("parent-rebuild");
    });
});
