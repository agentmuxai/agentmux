// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DRAG_SLOP_PX, installPaneSelectGuard, OWNER_ATTR, paneOf, ROOT_ATTR } from "./pane-select";

let uninstall: () => void;

const PANES = `
    <div id="chrome">tab strip</div>
    <div class="block" data-blockid="a" id="a">
        <div class="block-frame-default-header">header a</div>
        <div class="block-content"><p id="ta">text in a, first line</p><p id="ta2">text in a, second line</p><input id="fa" value="field"></div>
    </div>
    <div class="block" data-blockid="b" id="b">
        <div class="block-content"><p id="tb">text in b</p></div>
    </div>
    <div id="status">status bar</div>`;

function mount(html = PANES): void {
    document.body.innerHTML = html;
}

const el = (id: string) => document.getElementById(id)!;

function press(target: Element, init: { button?: number; pointerType?: string; shiftKey?: boolean } = {}): void {
    const ev = new MouseEvent("pointerdown", {
        bubbles: true,
        cancelable: true,
        button: init.button ?? 0,
        shiftKey: init.shiftKey ?? false,
        clientX: 100,
        clientY: 100,
    });
    Object.assign(ev, { pointerType: init.pointerType ?? "mouse" });
    target.dispatchEvent(ev);
}

function pointer(type: string, buttons: number, target: Element = document.body, dx = 40): void {
    target.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, buttons, clientX: 100 + dx, clientY: 100 }));
}

const locked = () => document.documentElement.hasAttribute(ROOT_ATTR);

function key(target: Element, init: KeyboardEventInit): KeyboardEvent {
    const ev = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
    target.dispatchEvent(ev);
    return ev;
}

/** jsdom queues `selectionchange`; let it run. */
const settle = () => new Promise<void>((r) => setTimeout(r, 0));

beforeEach(() => {
    mount();
    window.getSelection()?.removeAllRanges();
    uninstall = installPaneSelectGuard(window);
});

afterEach(() => {
    uninstall();
    document.body.innerHTML = "";
});

describe("a selection that starts in a pane (drag)", () => {
    it("locks the root and marks the pane that owns the press once the pointer moves past the slop", () => {
        press(el("ta"));
        expect(locked()).toBe(false);
        pointer("pointermove", 1, el("tb"));
        expect(locked()).toBe(true);
        expect(el("a").hasAttribute(OWNER_ATTR)).toBe(true);
        expect(el("b").hasAttribute(OWNER_ATTR)).toBe(false);
    });

    it("sets nothing while the drag stays inside its pane (a pane dragged by its header, text selected within it), so a busy window is not restyled", () => {
        const seen: string[] = [];
        const watch = new MutationObserver((rs) => rs.forEach((r) => seen.push(String(r.attributeName))));
        watch.observe(document.documentElement, { attributes: true });
        watch.observe(el("a"), { attributes: true });
        press(el("ta"));
        pointer("pointermove", 1, el("ta2"), 40);
        pointer("pointermove", 1, el("fa"), 90);
        pointer("pointermove", 1, el("a"), 140);
        expect(locked()).toBe(false);
        expect(el("a").hasAttribute(OWNER_ATTR)).toBe(false);
        expect(watch.takeRecords()).toEqual([]);
        expect(seen).toEqual([]);
        pointer("pointerup", 0, el("ta"));
        expect(watch.takeRecords()).toEqual([]);
        watch.disconnect();
    });

    it("locks as soon as the pointer is over anything but the pane: another pane, the chrome, the gap", () => {
        for (const outside of ["tb", "chrome", "status"]) {
            press(el("ta"));
            pointer("pointermove", 1, el("ta2"), 40);
            expect(locked()).toBe(false);
            pointer("pointermove", 1, el(outside), 80);
            expect(locked()).toBe(true);
            expect(el("a").hasAttribute(OWNER_ATTR)).toBe(true);
            pointer("pointerup", 0, el(outside));
            expect(locked()).toBe(false);
        }
    });

    it("sets nothing for a click, so a click never restyles the document", () => {
        const seen: string[] = [];
        const mo = new MutationObserver((rs) => rs.forEach((r) => seen.push(String(r.attributeName))));
        mo.observe(document.documentElement, { attributes: true });
        mo.observe(el("a"), { attributes: true });
        press(el("ta"));
        pointer("pointermove", 1, el("ta"), DRAG_SLOP_PX - 1);
        pointer("pointerup", 0, el("ta"), DRAG_SLOP_PX - 1);
        expect(mo.takeRecords().map((r) => r.attributeName)).toEqual([]);
        mo.disconnect();
        expect(seen).toEqual([]);
        expect(locked()).toBe(false);
    });

    it("releases on pointerup, pointercancel, dragstart, window blur, and a move with no button down", () => {
        const ends: Array<() => void> = [
            () => pointer("pointerup", 0, el("tb")),
            () => pointer("pointercancel", 0, el("tb")),
            () => el("ta").dispatchEvent(new Event("dragstart", { bubbles: true })),
            () => window.dispatchEvent(new Event("blur")),
            () => pointer("pointermove", 0, el("tb")),
        ];
        for (const end of ends) {
            press(el("ta"));
            pointer("pointermove", 1, el("tb"));
            expect(locked()).toBe(true);
            end();
            expect(locked()).toBe(false);
            expect(el("a").hasAttribute(OWNER_ATTR)).toBe(false);
        }
    });

    it("releases when the pane is closed under the drag", () => {
        press(el("ta"));
        pointer("pointermove", 1, el("tb"));
        const a = el("a");
        a.remove();
        pointer("pointermove", 1, el("tb"));
        expect(locked()).toBe(false);
        expect(a.hasAttribute(OWNER_ATTR)).toBe(false);
    });

    it("does not lock for a press outside every pane (the tab strip, the status bar), a right press, or a touch", () => {
        press(el("chrome"));
        pointer("pointermove", 1, el("tb"));
        expect(locked()).toBe(false);
        pointer("pointerup", 0);
        press(el("ta"), { button: 2 });
        pointer("pointermove", 1, el("tb"));
        expect(locked()).toBe(false);
        press(el("ta"), { pointerType: "touch" });
        pointer("pointermove", 1, el("tb"));
        expect(locked()).toBe(false);
    });

    it("sets nothing for a press on something that cannot start a selection (user-select: none), so a map or a button drag never restyles the document", () => {
        const real = window.getComputedStyle.bind(window);
        const spy = vi.spyOn(window, "getComputedStyle").mockImplementation((e: Element, pseudo?: string | null) => {
            const style = real(e, pseudo);
            return e.id === "tb" ? new Proxy(style, { get: (t, k) => (k === "userSelect" ? "none" : Reflect.get(t, k)) }) : style;
        });
        try {
            press(el("tb"));
            pointer("pointermove", 1, el("tb"));
            expect(locked()).toBe(false);
            expect(el("b").hasAttribute(OWNER_ATTR)).toBe(false);
            // A press on text that can be selected still locks.
            press(el("ta"));
            pointer("pointermove", 1, el("tb"));
            expect(locked()).toBe(true);
        } finally {
            spy.mockRestore();
        }
    });

    it("moves the ownership with a new press (a second drag in another pane)", () => {
        press(el("ta"));
        pointer("pointermove", 1, el("tb"));
        pointer("pointerup", 0);
        press(el("tb"));
        pointer("pointermove", 1, el("ta"));
        expect(el("b").hasAttribute(OWNER_ATTR)).toBe(true);
        expect(el("a").hasAttribute(OWNER_ATTR)).toBe(false);
    });

    it("finds the pane of a text node's element and nothing for chrome", () => {
        expect(paneOf(el("ta").firstChild)).toBe(el("a"));
        expect(paneOf(el("chrome"))).toBeNull();
    });
});

describe("Shift+click", () => {
    function select(from: Node, fromOffset: number, to: Node, toOffset: number): void {
        window.getSelection()!.setBaseAndExtent(from, fromOffset, to, toOffset);
    }

    it("drops a selection that began in another pane, so the click starts a new one", () => {
        select(el("ta").firstChild!, 2, el("ta").firstChild!, 6);
        press(el("tb"), { shiftKey: true });
        expect(window.getSelection()!.rangeCount).toBe(0);
    });

    it("drops a selection that began in a pane when the click is on the chrome", () => {
        select(el("ta").firstChild!, 2, el("ta").firstChild!, 6);
        press(el("status"), { shiftKey: true });
        expect(window.getSelection()!.rangeCount).toBe(0);
    });

    it("keeps a selection that stays in the pane it began in, and one with no pane", () => {
        select(el("ta").firstChild!, 2, el("ta").firstChild!, 6);
        press(el("ta2"), { shiftKey: true });
        expect(window.getSelection()!.rangeCount).toBe(1);
        select(el("chrome").firstChild!, 0, el("chrome").firstChild!, 3);
        press(el("tb"), { shiftKey: true });
        expect(window.getSelection()!.rangeCount).toBe(1);
    });

    it("leaves a plain click alone", () => {
        select(el("ta").firstChild!, 2, el("ta").firstChild!, 6);
        press(el("tb"));
        expect(window.getSelection()!.rangeCount).toBe(1);
    });
});

describe("a selection extended from the keyboard", () => {
    it("is put back at the pane's content end when its far end leaves the pane forward", async () => {
        const sel = window.getSelection()!;
        sel.setBaseAndExtent(el("ta").firstChild!, 3, el("tb").firstChild!, 4);
        await settle();
        expect(sel.anchorNode).toBe(el("ta").firstChild);
        expect(sel.anchorOffset).toBe(3);
        expect(el("a").contains(sel.focusNode)).toBe(true);
        expect(el("b").contains(sel.focusNode)).toBe(false);
        expect(sel.toString()).not.toContain("text in b");
    });

    it("is put back at the pane's content start when its far end leaves the pane backward", async () => {
        const sel = window.getSelection()!;
        sel.setBaseAndExtent(el("tb").firstChild!, 3, el("chrome").firstChild!, 1);
        await settle();
        expect(el("b").contains(sel.focusNode)).toBe(true);
        expect(sel.toString()).not.toContain("tab strip");
    });

    it("leaves a selection inside one pane, and a selection that began outside every pane, alone", async () => {
        const sel = window.getSelection()!;
        sel.setBaseAndExtent(el("ta").firstChild!, 1, el("ta2").firstChild!, 4);
        await settle();
        expect(sel.focusNode).toBe(el("ta2").firstChild);
        sel.setBaseAndExtent(el("chrome").firstChild!, 1, el("tb").firstChild!, 4);
        await settle();
        expect(sel.focusNode).toBe(el("tb").firstChild);
    });

    it("does not patch while a press is held (the stylesheet confines a drag)", async () => {
        const sel = window.getSelection()!;
        press(el("ta"));
        sel.setBaseAndExtent(el("ta").firstChild!, 3, el("tb").firstChild!, 4);
        await settle();
        expect(sel.focusNode).toBe(el("tb").firstChild);
    });
});

describe("Ctrl+A", () => {
    it("selects the pane's own content, from a key that lands on the pane's focus anchor", () => {
        mount(PANES.replace('<input id="fa" value="field">', '<input id="fa" class="dummy-focus" value="">'));
        const ev = key(el("fa"), { key: "a", ctrlKey: true });
        expect(ev.defaultPrevented).toBe(true);
        const sel = window.getSelection()!;
        expect(sel.toString()).toContain("text in a, first line");
        expect(sel.toString()).not.toContain("text in b");
        expect(sel.toString()).not.toContain("tab strip");
        expect(sel.toString()).not.toContain("header a");
    });

    it("selects the pane's content when the key lands inside the pane's text", () => {
        key(el("ta"), { key: "A", ctrlKey: true });
        expect(window.getSelection()!.toString()).toContain("second line");
        expect(window.getSelection()!.toString()).not.toContain("text in b");
    });

    it("selects the focused pane's content when the key lands on the body", () => {
        el("b").classList.add("block-focused");
        const ev = key(document.body, { key: "a", ctrlKey: true });
        expect(ev.defaultPrevented).toBe(true);
        expect(window.getSelection()!.toString()).toBe("text in b");
    });

    it("does nothing with no focused pane, in a text field, the terminal, the Editor or a contenteditable", () => {
        expect(key(document.body, { key: "a", ctrlKey: true }).defaultPrevented).toBe(false);
        expect(key(el("fa"), { key: "a", ctrlKey: true }).defaultPrevented).toBe(false);
        for (const cls of ["xterm", "monaco-editor"]) {
            el("ta").className = cls;
            expect(key(el("ta"), { key: "a", ctrlKey: true }).defaultPrevented).toBe(false);
        }
        el("ta").className = "";
        el("ta").setAttribute("contenteditable", "true");
        expect(key(el("ta"), { key: "a", ctrlKey: true }).defaultPrevented).toBe(false);
    });

    it("does nothing for other keys and modifiers, or a key a command already took", () => {
        expect(key(el("ta"), { key: "b", ctrlKey: true }).defaultPrevented).toBe(false);
        expect(key(el("ta"), { key: "a" }).defaultPrevented).toBe(false);
        expect(key(el("ta"), { key: "a", ctrlKey: true, shiftKey: true }).defaultPrevented).toBe(false);
        expect(key(el("ta"), { key: "a", metaKey: true }).defaultPrevented).toBe(false);
        const taken = (e: Event) => e.preventDefault();
        window.addEventListener("keydown", taken, true);
        window.getSelection()!.removeAllRanges();
        key(el("ta"), { key: "a", ctrlKey: true });
        window.removeEventListener("keydown", taken, true);
        expect(window.getSelection()!.rangeCount).toBe(0);
    });
});

describe("uninstall", () => {
    it("clears the lock and stops listening", () => {
        press(el("ta"));
        pointer("pointermove", 1, el("tb"));
        uninstall();
        expect(locked()).toBe(false);
        press(el("ta"));
        pointer("pointermove", 1, el("tb"));
        expect(locked()).toBe(false);
        uninstall = installPaneSelectGuard(window);
    });
});
