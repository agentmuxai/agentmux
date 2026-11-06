// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/**
 * The JS overlay context menu, one behaviour per test: how a row is drawn for each kind of item,
 * what a click, Escape and an outside press do, where focus goes when the menu closes, how the
 * menu and its submenus are placed (and what happens when placement fails or arrives late), how
 * submenu hover is wired to its rows, and what a live update keeps. The pane-overlay holes, the
 * placement and the submenu hover are replaced with recorders, so each test sees the arguments and
 * the order of the calls the menu makes.
 */
import type { MenuPositionResult } from "@/app/util/menu-position";
import type { SubmenuHoverOptions } from "@/app/util/submenu-hover";
import { Z_JS_MENU } from "@/app/zindex";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";

type FakeHover = {
    opts: SubmenuHoverOptions;
    onTriggerEnter: Mock;
    onTriggerLeave: Mock;
    onSubmenuEnter: Mock;
    onSubmenuLeave: Mock;
    setSubmenuEl: Mock;
    close: Mock;
    openNow: Mock;
    dispose: Mock;
};
type PlaceCall = {
    req: { anchor: unknown; placement: string; avoidNativePanes?: boolean };
    el: HTMLElement;
    positionAtCall: string;
};

const h = vi.hoisted(() => ({
    regs: [] as { el: HTMLElement; opts: { keys?: boolean } | undefined; releases: number }[],
    places: [] as PlaceCall[],
    mode: "ok" as "ok" | "reject" | "hold",
    held: [] as { resolve: (r: MenuPositionResult) => void; reject: (e: Error) => void }[],
    asserts: [] as { el: HTMLElement; label: string }[],
    hovers: [] as FakeHover[],
    placed: {
        style: { left: "12px", top: "34px" },
        placement: "bottom-start",
        maxHeight: 456,
        maxWidth: 300,
    } as MenuPositionResult,
}));

vi.mock("@/app/platform/pane-overlay", () => ({
    registerPaneOverlay: (el: HTMLElement, opts?: { keys?: boolean }) => {
        const reg = { el, opts, releases: 0 };
        h.regs.push(reg);
        return {
            update: () => {},
            release: () => {
                reg.releases++;
            },
        };
    },
}));
vi.mock("@/app/util/menu-position", () => ({
    assertMenuInPaintableArea: (el: HTMLElement, label: string) => {
        h.asserts.push({ el, label });
    },
    computeMenuPosition: (req: PlaceCall["req"], el: HTMLElement) => {
        h.places.push({ req, el, positionAtCall: el.style.position });
        if (h.mode === "reject") return Promise.reject(new Error("no placement"));
        if (h.mode === "hold") {
            return new Promise<MenuPositionResult>((resolve, reject) => h.held.push({ resolve, reject }));
        }
        return Promise.resolve(h.placed);
    },
}));
vi.mock("@/app/util/submenu-hover", () => ({
    createSubmenuHover: (opts: SubmenuHoverOptions) => {
        const hover: FakeHover = {
            opts,
            onTriggerEnter: vi.fn(),
            onTriggerLeave: vi.fn(),
            onSubmenuEnter: vi.fn(),
            onSubmenuLeave: vi.fn(),
            setSubmenuEl: vi.fn(),
            close: vi.fn(),
            openNow: vi.fn(),
            dispose: vi.fn(),
        };
        h.hovers.push(hover);
        return hover;
    },
}));

import {
    closeJsContextMenu,
    isJsContextMenuOpen,
    jsContextMenuApi,
    onJsContextMenuClosed,
    showJsContextMenu,
    updateJsContextMenu,
} from "./js-context-menu";

let frames: FrameRequestCallback[] = [];
const flushFrames = () => {
    const due = frames;
    frames = [];
    for (const cb of due) cb(0);
};
const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
    h.regs.length = 0;
    h.places.length = 0;
    h.held.length = 0;
    h.asserts.length = 0;
    h.hovers.length = 0;
    h.mode = "ok";
    frames = [];
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
        frames.push(cb);
        return frames.length;
    });
});
afterEach(() => {
    closeJsContextMenu();
    document.body.replaceChildren();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
});

const item = (label: string, extra: Partial<NativeContextMenuItem> = {}): NativeContextMenuItem => ({
    id: label,
    label,
    ...extra,
});
const withSub = (label: string, submenu: NativeContextMenuItem[], extra: Partial<NativeContextMenuItem> = {}) =>
    item(label, { type: "submenu", submenu, ...extra });
const show = (items: NativeContextMenuItem[], onClick: ((id: string) => void) | null = null, at = { x: 7, y: 9 }) =>
    showJsContextMenu(items, at, onClick);
const overlay = () => document.getElementById("cef-context-menu-overlay")!;
// A level is an unzoomed shell (placed, hidden, registered as a pane overlay) around a body (the rows). The menu's
// shell is the overlay's first child; every submenu shell is a sibling after it, in the order of the rows that open
// them, not nested in those rows.
const menuShell = () => overlay().firstElementChild as HTMLElement;
const menuEl = () => menuShell().firstElementChild as HTMLElement;
const rowAt = (i: number) => menuEl().children[i] as HTMLElement;
const subShells = () => [...overlay().children].slice(1) as HTMLElement[];
const subOf = (row: HTMLElement) => {
    const opening = [...row.parentElement!.children].filter((r) => r.querySelector(":scope > .fa-chevron-right"));
    return subShells()[opening.indexOf(row)];
};
const bodyOf = (shell: HTMLElement) => shell.firstElementChild as HTMLElement;

describe("menu state and close listeners", () => {
    it("reports whether a menu is open", () => {
        expect(isJsContextMenuOpen()).toBe(false);
        show([item("A")]);
        expect(isJsContextMenuOpen()).toBe(true);
        closeJsContextMenu();
        expect(isJsContextMenuOpen()).toBe(false);
    });

    it("stops calling a listener once it unsubscribes", () => {
        const listener = vi.fn();
        const stop = onJsContextMenuClosed(listener);
        stop();
        show([item("A")]);
        closeJsContextMenu();
        expect(listener).not.toHaveBeenCalled();
    });

    it("runs only the listeners that were there when the menu started to close", () => {
        const late = vi.fn();
        const stops: (() => void)[] = [];
        stops.push(onJsContextMenuClosed(() => stops.push(onJsContextMenuClosed(late))));
        show([item("A")]);
        closeJsContextMenu();
        expect(late).not.toHaveBeenCalled();
        for (const stop of stops) stop();
    });

    it("lets an older menu's late close leave the open menu alone", () => {
        show([item("A")]);
        const stale = rowAt(0);
        show([item("B")]);
        stale.click();
        expect(isJsContextMenuOpen()).toBe(true);
        expect(overlay().isConnected).toBe(true);
        expect(updateJsContextMenu([item("C")])).toBe(true);
    });
});

describe("document listeners", () => {
    it("listens for presses and keys on the capture phase, and removes both on close", () => {
        const add = vi.spyOn(document, "addEventListener");
        const remove = vi.spyOn(document, "removeEventListener");
        show([item("A")]);
        const down = add.mock.calls.find((c) => c[0] === "mousedown")!;
        const key = add.mock.calls.find((c) => c[0] === "keydown")!;
        expect(down[2]).toBe(true);
        expect(key[2]).toBe(true);
        closeJsContextMenu();
        expect(remove).toHaveBeenCalledWith("mousedown", down[1], true);
        expect(remove).toHaveBeenCalledWith("keydown", key[1], true);
    });

    it("treats a press whose target is not a node as outside", () => {
        const add = vi.spyOn(document, "addEventListener");
        show([item("A")]);
        const down = add.mock.calls.find((c) => c[0] === "mousedown")![1] as (e: MouseEvent) => void;
        down({ target: null } as unknown as MouseEvent);
        expect(isJsContextMenuOpen()).toBe(false);
    });

    it("lets a press inside the menu through, and closes on one outside it", () => {
        show([item("A")]);
        rowAt(0).dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
        expect(isJsContextMenuOpen()).toBe(true);
        document.body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
        expect(isJsContextMenuOpen()).toBe(false);
    });

    it("takes Escape for itself: cancels it, stops it, and closes", () => {
        const seen = vi.fn();
        window.addEventListener("keydown", seen);
        show([item("A")]);
        const esc = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
        document.body.dispatchEvent(esc);
        window.removeEventListener("keydown", seen);
        expect(esc.defaultPrevented).toBe(true);
        expect(seen).not.toHaveBeenCalled();
        expect(isJsContextMenuOpen()).toBe(false);
    });

    it("ignores any other key", () => {
        const seen = vi.fn();
        window.addEventListener("keydown", seen);
        show([item("A")]);
        const key = new KeyboardEvent("keydown", { key: "a", bubbles: true, cancelable: true });
        document.body.dispatchEvent(key);
        window.removeEventListener("keydown", seen);
        expect(key.defaultPrevented).toBe(false);
        expect(seen).toHaveBeenCalledTimes(1);
        expect(isJsContextMenuOpen()).toBe(true);
    });
});

describe("focus when the menu closes", () => {
    let input: HTMLTextAreaElement;
    beforeEach(() => {
        input = document.createElement("textarea");
        document.body.appendChild(input);
        input.focus();
    });

    it("waits until the chosen item has run, then hands focus back", async () => {
        let during: Element | null = null;
        show([item("Paste")], () => {
            during = document.activeElement;
        });
        input.blur();
        rowAt(0).click();
        expect(during).toBe(document.body);
        await Promise.resolve();
        expect(document.activeElement).toBe(input);
    });

    it("asks the opener not to scroll", async () => {
        const focus = vi.spyOn(input, "focus");
        show([item("Paste")]);
        input.blur();
        closeJsContextMenu();
        await Promise.resolve();
        expect(focus).toHaveBeenCalledWith({ preventScroll: true });
    });

    it("hands focus back when the page reports no focused element at all", async () => {
        show([item("Paste")]);
        const focus = vi.spyOn(input, "focus");
        vi.spyOn(document, "activeElement", "get").mockReturnValue(null);
        closeJsContextMenu();
        await Promise.resolve();
        expect(focus).toHaveBeenCalled();
    });

    it("hands focus back when it was inside the menu that went away", async () => {
        show([item("Paste")]);
        const focus = vi.spyOn(input, "focus");
        vi.spyOn(document, "activeElement", "get").mockReturnValue(rowAt(0));
        closeJsContextMenu();
        await Promise.resolve();
        expect(focus).toHaveBeenCalled();
    });

    it("never focuses an opener that is no longer in the page", async () => {
        const focus = vi.spyOn(input, "focus");
        show([item("Paste")], () => input.remove());
        input.blur();
        rowAt(0).click();
        await Promise.resolve();
        expect(focus).not.toHaveBeenCalled();
    });

    it("does not treat the page body as an opener to give focus back to", async () => {
        input.blur();
        const body = vi.spyOn(document.body, "focus");
        show([item("Paste")]);
        closeJsContextMenu();
        await Promise.resolve();
        expect(body).not.toHaveBeenCalled();
    });
});

describe("the overlay and the menu element", () => {
    it("covers the window above everything the menu sits over", () => {
        show([item("A")]);
        expect(overlay().style.position).toBe("fixed");
        expect(["0", "0px"]).toContain(overlay().style.inset);
        expect(overlay().style.zIndex).toBe(String(Z_JS_MENU));
    });

    it("replaces an overlay left in the page", () => {
        const stray = document.createElement("div");
        stray.id = "cef-context-menu-overlay";
        document.body.appendChild(stray);
        show([item("A")]);
        expect(document.querySelectorAll("#cef-context-menu-overlay").length).toBe(1);
        expect(stray.isConnected).toBe(false);
    });

    it("starts the menu hidden at the cursor, marked as a pane overlay, until it is placed", async () => {
        show([item("A")]);
        const el = menuShell();
        expect(el.className).toBe("anchored-popover anchored-popover--menu");
        expect(menuEl().className).toBe("menu anchored-popover-body anchored-popover-body--chrome-zoom");
        expect(el.hasAttribute("data-pane-overlay")).toBe(true);
        expect(el.style.left).toBe("7px");
        expect(el.style.top).toBe("9px");
        expect(el.style.visibility).toBe("hidden");
        await settle();
        expect(el.style.visibility).toBe("");
    });

    it("registers the menu and its submenu as keyed pane overlays", () => {
        show([withSub("Group", [item("One")])]);
        const menuReg = h.regs.find((r) => r.el === menuShell())!;
        const subReg = h.regs.find((r) => r.el === subOf(rowAt(0)))!;
        expect(menuReg.opts).toEqual({ keys: true });
        expect(subReg.opts).toEqual({ keys: true });
    });
});

describe("placing the menu", () => {
    it("asks for the spot below the cursor, and lets the menu cover a browser pane", () => {
        show([item("A")]);
        expect(h.places.length).toBe(1);
        expect(h.places[0].el).toBe(menuShell());
        expect(h.places[0].req).toEqual({ anchor: { x: 7, y: 9 }, placement: "bottom-start", avoidNativePanes: false });
    });

    it("applies the computed spot and size cap, then reveals and checks the menu", async () => {
        show([item("A")]);
        await settle();
        const s = menuShell().style;
        expect([s.position, s.left, s.top, s.visibility]).toEqual(["fixed", "12px", "34px", ""]);
        // The cap is on the body, in its own zoomed lengths: the free height divided by the chrome zoom.
        const b = menuEl().style;
        expect([b.maxHeight, b.overflowY]).toEqual(["calc(456px / var(--zoomfactor, 1))", "auto"]);
        expect(h.asserts).toEqual([{ el: menuShell(), label: "context-menu" }]);
    });

    it("does not place or check a menu that closed while placement was pending", async () => {
        h.mode = "hold";
        show([item("A")]);
        const el = menuEl();
        closeJsContextMenu();
        h.held[0].resolve(h.placed);
        await settle();
        expect(el.style.maxHeight).toBe("");
        expect(h.asserts).toEqual([]);
    });

    it("still reveals the menu at the cursor when placement fails", async () => {
        h.mode = "reject";
        show([item("A")]);
        await settle();
        expect(menuShell().style.visibility).toBe("");
        expect(menuShell().style.left).toBe("7px");
    });

    it("leaves a menu that closed before placement failed alone", async () => {
        h.mode = "hold";
        show([item("A")]);
        const el = menuShell();
        closeJsContextMenu();
        h.held[0].reject(new Error("late"));
        await settle();
        expect(el.style.visibility).toBe("hidden");
    });
});

describe("rows", () => {
    it("draws a separator as a divider, not a row, and does not let it reserve the icon slot", () => {
        show([item("A"), item("", { type: "separator", icon: "copy" }), item("B")]);
        expect([...menuEl().children].map((c) => c.className)).toEqual(["menu-item", "menu-divider", "menu-item"]);
        expect(menuEl().querySelectorAll(".menu-item-icon").length).toBe(0);
    });

    it("draws nothing for a hidden item", () => {
        show([item("A"), item("B", { visible: false }), item("C")]);
        expect([...menuEl().querySelectorAll(".label")].map((l) => l.textContent)).toEqual(["A", "C"]);
    });

    it("dims a disabled row and takes it out of the pointer's way", () => {
        show([item("On"), item("Off", { enabled: false })]);
        const on = rowAt(0).style;
        const off = rowAt(1).style;
        expect([on.cursor, on.opacity, on.pointerEvents]).toEqual(["", "", ""]);
        expect([off.cursor, off.opacity, off.pointerEvents]).toEqual(["default", "0.4", "none"]);
    });

    it("does nothing when a disabled row is clicked or entered", () => {
        const onClick = vi.fn();
        show([withSub("Group", [item("One")]), item("Off", { enabled: false })], onClick);
        rowAt(1).dispatchEvent(new MouseEvent("mouseenter"));
        rowAt(1).click();
        expect(onClick).not.toHaveBeenCalled();
        expect(isJsContextMenuOpen()).toBe(true);
        expect(h.hovers[0].close).not.toHaveBeenCalled();
    });

    it("gives radio and checkbox rows a check slot, filled only when checked", () => {
        show([
            item("Radio", { type: "radio" }),
            item("Box", { type: "checkbox" }),
            item("On", { type: "checkbox", checked: true }),
            item("Plain off", { checked: false }),
            item("Plain"),
        ]);
        const slot = (i: number) => rowAt(i).querySelector(".menu-item-check");
        expect(slot(0)!.className).toBe("fa-solid fa-fw menu-item-icon menu-item-check");
        expect(slot(1)!.className).toBe("fa-solid fa-fw menu-item-icon menu-item-check");
        expect(slot(2)!.className).toBe("fa-solid fa-fw fa-check menu-item-icon menu-item-check");
        expect(slot(3)!.className).toBe("fa-solid fa-fw menu-item-icon menu-item-check");
        expect(slot(4)).toBeNull();
    });

    it("paints a swatch row's colour", () => {
        show([item("Blue", { swatchColor: "#3366ff" })]);
        expect(rowAt(0).querySelector<HTMLElement>(".menu-item-swatch")!.style.backgroundColor).toBe(
            "rgb(51, 102, 255)"
        );
    });

    it("draws an empty label for an item with none", () => {
        show([{ id: "x" } as NativeContextMenuItem]);
        expect(rowAt(0).querySelector(".label")!.textContent).toBe("");
    });

    it("closes and reports the id when a row is clicked", () => {
        const onClick = vi.fn();
        show([item("A")], onClick);
        rowAt(0).click();
        expect(onClick).toHaveBeenCalledWith("A");
        expect(isJsContextMenuOpen()).toBe(false);
    });

    it("closes without reporting when the row has no id", () => {
        const onClick = vi.fn();
        show([{ label: "Anon" } as NativeContextMenuItem], onClick);
        rowAt(0).click();
        expect(onClick).not.toHaveBeenCalled();
        expect(isJsContextMenuOpen()).toBe(false);
    });

    it("closes without a callback to report to", () => {
        show([item("A")], null);
        rowAt(0).click();
        expect(isJsContextMenuOpen()).toBe(false);
    });

    it("treats a row with an empty submenu as a plain row", () => {
        const onClick = vi.fn();
        show([item("Empty", { submenu: [] })], onClick);
        expect(menuEl().querySelector(".sub-menu")).toBeNull();
        expect(menuEl().querySelector(".fa-chevron-right")).toBeNull();
        rowAt(0).click();
        expect(onClick).toHaveBeenCalledWith("Empty");
    });

    it("draws a submenu row with a chevron and a nested, hidden, pane-overlay submenu of its rows", () => {
        show([withSub("Group", [item("One"), item("Two")])]);
        const sub = subOf(rowAt(0));
        expect(rowAt(0).querySelector("i")!.className).toBe("fa-solid fa-chevron-right");
        expect(sub.hasAttribute("data-pane-overlay")).toBe(true);
        expect([...sub.querySelectorAll(".label")].map((l) => l.textContent)).toEqual(["One", "Two"]);
    });
});

describe("submenu hover wiring", () => {
    const twoSubs = () => show([withSub("A", [item("a1")]), item("Plain"), withSub("B", [item("b1")])]);

    it("hands each submenu to its hover controller, one that stays open when the pointer leaves", () => {
        twoSubs();
        expect(h.hovers.length).toBe(2);
        expect(h.hovers[0].setSubmenuEl).toHaveBeenCalledWith(subOf(rowAt(0)));
        expect(h.hovers[1].setSubmenuEl).toHaveBeenCalledWith(subOf(rowAt(2)));
        expect(h.hovers[0].opts.closeOnLeave).toBe(false);
    });

    it("closes the other submenus on entering a submenu row, but not its own", () => {
        twoSubs();
        rowAt(2).dispatchEvent(new MouseEvent("mouseenter"));
        expect(h.hovers[0].close).toHaveBeenCalledTimes(1);
        expect(h.hovers[1].close).not.toHaveBeenCalled();
        expect(h.hovers[1].onTriggerEnter).toHaveBeenCalledTimes(1);
        rowAt(0).dispatchEvent(new MouseEvent("mouseenter"));
        expect(h.hovers[1].close).toHaveBeenCalledTimes(1);
        expect(h.hovers[0].close).toHaveBeenCalledTimes(1);
        expect(h.hovers[0].onTriggerEnter).toHaveBeenCalledTimes(1);
    });

    it("closes every open submenu on entering a plain row", () => {
        twoSubs();
        rowAt(1).dispatchEvent(new MouseEvent("mouseenter"));
        expect(h.hovers[0].close).toHaveBeenCalledTimes(1);
        expect(h.hovers[1].close).toHaveBeenCalledTimes(1);
    });

    it("keeps the levels apart: entering a row in a submenu closes only that level's peers", () => {
        show([withSub("A", [withSub("Inner", [item("x")]), item("Plain")]), withSub("B", [item("b1")])]);
        const innerPlain = bodyOf(subOf(rowAt(0))).children[1] as HTMLElement;
        innerPlain.dispatchEvent(new MouseEvent("mouseenter", { bubbles: false }));
        const [inner, a, b] = h.hovers;
        expect(inner.close).toHaveBeenCalledTimes(1);
        expect(a.close).not.toHaveBeenCalled();
        expect(b.close).not.toHaveBeenCalled();
    });

    it("forwards pointer movement on the row and on the submenu to the controller", () => {
        twoSubs();
        const row = rowAt(0);
        const sub = subOf(row);
        const leave = new MouseEvent("mouseleave");
        row.dispatchEvent(leave);
        expect(h.hovers[0].onTriggerLeave).toHaveBeenCalledWith(leave);
        sub.dispatchEvent(new MouseEvent("mouseenter"));
        expect(h.hovers[0].onSubmenuEnter).toHaveBeenCalledTimes(1);
        const subLeave = new MouseEvent("mouseleave");
        sub.dispatchEvent(subLeave);
        expect(h.hovers[0].onSubmenuLeave).toHaveBeenCalledWith(subLeave);
    });

    it("shows the submenu hidden on open, and hides it on close", () => {
        twoSubs();
        const sub = subOf(rowAt(0));
        h.hovers[0].opts.onOpen();
        expect([sub.style.display, sub.style.visibility]).toEqual(["", "hidden"]);
        h.hovers[0].opts.onClose();
        expect(sub.style.display).toBe("none");
    });

    it("places the submenu to the right of its row, as a fixed element, a frame after it opens", async () => {
        twoSubs();
        const row = rowAt(0);
        const rect = {
            left: 1,
            top: 2,
            right: 3,
            bottom: 4,
            width: 2,
            height: 2,
            x: 1,
            y: 2,
            toJSON: () => ({}),
        } as DOMRect;
        vi.spyOn(row, "getBoundingClientRect").mockReturnValue(rect);
        const sub = subOf(row);
        const before = h.places.length;
        h.hovers[0].opts.onOpen();
        expect(h.places.length).toBe(before);
        flushFrames();
        const call = h.places[before];
        expect(call.el).toBe(sub);
        expect(call.positionAtCall).toBe("fixed");
        expect(call.req.anchor).toBe(rect);
        expect(call.req.placement).toBe("right-start");
        expect(call.req.avoidNativePanes).toBe(false);
        await settle();
        const s = sub.style;
        expect([s.position, s.left, s.top, s.visibility]).toEqual(["fixed", "12px", "34px", ""]);
        expect([bodyOf(sub).style.maxHeight, bodyOf(sub).style.overflowY]).toEqual(["calc(456px / var(--zoomfactor, 1))", "auto"]);
        expect(h.asserts).toContainEqual({ el: sub, label: "context-submenu" });
    });

    it("does not place a submenu that was hidden again before the frame", () => {
        twoSubs();
        const before = h.places.length;
        h.hovers[0].opts.onOpen();
        subOf(rowAt(0)).style.display = "none";
        flushFrames();
        expect(h.places.length).toBe(before);
    });

    it("does not place a submenu whose menu closed before the frame", () => {
        twoSubs();
        const before = h.places.length;
        h.hovers[0].opts.onOpen();
        closeJsContextMenu();
        flushFrames();
        expect(h.places.length).toBe(before);
    });

    it("leaves a submenu hidden again while placement was in flight where it was", async () => {
        h.mode = "hold";
        twoSubs();
        const sub = subOf(rowAt(0));
        h.hovers[0].opts.onOpen();
        flushFrames();
        sub.style.display = "none";
        h.held[h.held.length - 1].resolve(h.placed);
        await settle();
        expect(sub.style.left).toBe("0px");
        expect(h.asserts.some((a) => a.el === sub)).toBe(false);
    });

    it("leaves a submenu whose menu closed while placement was in flight where it was", async () => {
        h.mode = "hold";
        twoSubs();
        const sub = subOf(rowAt(0));
        h.hovers[0].opts.onOpen();
        flushFrames();
        closeJsContextMenu();
        h.held[h.held.length - 1].resolve(h.placed);
        await settle();
        expect(sub.style.left).toBe("0px");
        expect(h.asserts.some((a) => a.el === sub)).toBe(false);
    });

    it("falls back to the row's right edge, in real pixels, and shows the submenu, when its placement fails", async () => {
        h.mode = "reject";
        twoSubs();
        const sub = subOf(rowAt(0));
        h.hovers[0].opts.onOpen();
        flushFrames();
        await settle();
        const s = sub.style;
        // jsdom gives the row an empty rect at the origin: its right edge and top are 0.
        expect([s.position, s.left, s.top, s.visibility]).toEqual(["fixed", "0px", "0px", ""]);
    });

    it("leaves a submenu whose menu closed before its placement failed untouched", async () => {
        h.mode = "hold";
        twoSubs();
        const sub = subOf(rowAt(0));
        h.hovers[0].opts.onOpen();
        flushFrames();
        closeJsContextMenu();
        h.held[h.held.length - 1].reject(new Error("late"));
        await settle();
        expect([sub.style.position, sub.style.visibility]).toEqual(["fixed", "hidden"]);
    });
});

describe("updating an open menu", () => {
    it("keeps a submenu that was open open, on the new row of the same label", () => {
        show([withSub("A", [item("a1")]), item("Plain")]);
        subOf(rowAt(0)).style.display = "";
        updateJsContextMenu([withSub("A", [item("a2")]), item("Plain")]);
        expect(h.hovers[0].dispose).toHaveBeenCalledTimes(1);
        expect(h.hovers.length).toBe(2);
        expect(h.hovers[1].openNow).toHaveBeenCalledTimes(1);
        // The old submenu shell is gone and the new one is a sibling of the menu, not nested in a row.
        expect(subShells().length).toBe(1);
        expect(subShells()[0].querySelector(".label")!.textContent).toBe("a2");
    });

    it("leaves a submenu that was closed closed", () => {
        show([withSub("A", [item("a1")])]);
        updateJsContextMenu([withSub("A", [item("a2")])]);
        expect(h.hovers[1].openNow).not.toHaveBeenCalled();
    });

    it("opens nothing when the submenu that was open is gone", () => {
        show([withSub("A", [item("a1")])]);
        subOf(rowAt(0)).style.display = "";
        updateJsContextMenu([withSub("B", [item("b1")])]);
        expect(h.hovers[1].openNow).not.toHaveBeenCalled();
    });

    it("tracks only the menu's own submenu rows, not those nested deeper", () => {
        show([withSub("A", [withSub("Inner", [item("x")])])]);
        const [inner, outer] = h.hovers;
        updateJsContextMenu([item("Plain")]);
        expect(outer.dispose).toHaveBeenCalledTimes(1);
        expect(inner.dispose).not.toHaveBeenCalled();
    });

    it("does not track a submenu row that has no label", () => {
        show([withSub("", [item("x")])]);
        updateJsContextMenu([item("Plain")]);
        expect(h.hovers[0].dispose).not.toHaveBeenCalled();
    });

    it("disposes each controller once, however many updates follow", () => {
        show([withSub("A", [item("a1")])]);
        updateJsContextMenu([withSub("B", [item("b1")])]);
        updateJsContextMenu([withSub("C", [item("c1")])]);
        expect(h.hovers[0].dispose).toHaveBeenCalledTimes(1);
        expect(h.hovers[1].dispose).toHaveBeenCalledTimes(1);
    });

    it("clears the old height and places the new list again", async () => {
        show([item("A")]);
        await settle();
        expect(menuEl().style.maxHeight).toBe("calc(456px / var(--zoomfactor, 1))");
        h.mode = "hold";
        const before = h.places.length;
        updateJsContextMenu([item("A"), item("B")]);
        expect(menuEl().style.maxHeight).toBe("");
        expect(h.places.length).toBe(before + 1);
        expect(h.places[before].el).toBe(menuShell());
    });

    it("draws the new items", () => {
        show([item("A")]);
        updateJsContextMenu([item("X"), item("Y")]);
        expect([...menuEl().querySelectorAll(".label")].map((l) => l.textContent)).toEqual(["X", "Y"]);
    });

    it("leaves a menu that was taken out of the page alone", () => {
        show([item("A")]);
        const el = menuEl();
        overlay().remove();
        expect(updateJsContextMenu([item("B")])).toBe(true);
        expect([...el.querySelectorAll(".label")].map((l) => l.textContent)).toEqual(["A"]);
    });
});

describe("jsContextMenuApi", () => {
    it("shows nothing for no menu or an empty one", () => {
        const api = jsContextMenuApi();
        expect(() => api.showContextMenu("w", undefined)).not.toThrow();
        api.showContextMenu("w", []);
        expect(document.getElementById("cef-context-menu-overlay")).toBeNull();
    });

    it("opens a menu at the origin when no position is given", () => {
        jsContextMenuApi().showContextMenu("w", [item("A")]);
        expect([menuShell().style.left, menuShell().style.top]).toEqual(["0px", "0px"]);
    });

    it("opens a menu at the position it is given", () => {
        jsContextMenuApi().showContextMenu("w", [item("A")], { x: 21, y: 22 });
        expect([menuShell().style.left, menuShell().style.top]).toEqual(["21px", "22px"]);
    });

    it("reports a clicked row to the registered callback", () => {
        const api = jsContextMenuApi();
        const cb = vi.fn();
        api.onContextMenuClick(cb);
        api.showContextMenu("w", [item("A")]);
        rowAt(0).click();
        expect(cb).toHaveBeenCalledWith("A");
    });
});

describe("chrome zoom (task #23943)", () => {
    it("draws every level as an unzoomed shell around a zoomed body, all siblings in the overlay", () => {
        show([withSub("A", [withSub("Inner", [item("x")])]), item("Plain")]);
        const shells = [...overlay().children] as HTMLElement[];
        expect(shells.length).toBe(3); // the menu, A's submenu, Inner's submenu
        for (const shell of shells) {
            expect(shell.classList.contains("anchored-popover")).toBe(true);
            expect(bodyOf(shell).classList.contains("anchored-popover-body--chrome-zoom")).toBe(true);
            // The shell is never zoomed itself (that would apply the factor twice) and never nested in a row,
            // where a zoomed body would scale its fixed left/top a second time.
            expect(shell.className).not.toContain("chrome-zoom");
            expect(shell.parentElement).toBe(overlay());
            expect(shell.closest(".menu-item")).toBeNull();
        }
        expect(shells.map((shell) => bodyOf(shell).classList.contains("sub-menu"))).toEqual([false, true, true]);
    });

    it("closes a submenu's own submenus with it, since they are no longer hidden by its display", () => {
        show([withSub("A", [withSub("Inner", [item("x")])])]);
        const [inner, a] = h.hovers;
        a.opts.onClose();
        expect(subOf(rowAt(0)).style.display).toBe("none");
        expect(inner.close).toHaveBeenCalledTimes(1);
    });

    it("stacks a submenu above the level that opened it", () => {
        show([withSub("A", [withSub("Inner", [item("x")])])]);
        const order = [...overlay().children];
        expect(order.indexOf(subShells()[0])).toBeLessThan(order.indexOf(subShells()[1]));
    });
});
