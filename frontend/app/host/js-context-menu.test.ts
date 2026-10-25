// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/**
 * The icon slot of the JS overlay context menu (SPEC_CONTEXT_MENU_ICONS_2026_10_25.md §3): a row
 * with an icon draws it in the leading slot, a menu level where any row has one reserves the slot
 * on every row so labels line up, and a level with none is drawn as it was.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/platform/pane-overlay", () => ({ registerPaneOverlay: () => ({ release: () => {} }) }));
vi.mock("@/app/util/menu-position", () => ({
    assertMenuInPaintableArea: () => {},
    computeMenuPosition: () => Promise.resolve({ style: { left: 0, top: 0 }, maxHeight: 600 }),
}));

import { closeJsContextMenu, showJsContextMenu } from "./js-context-menu";

afterEach(() => {
    closeJsContextMenu();
    document.body.replaceChildren();
});

const row = (label: string): HTMLElement =>
    [...document.querySelectorAll<HTMLElement>(".menu-item")].find((r) => r.querySelector(".label")?.textContent === label)!;
const slot = (label: string): HTMLElement | null => row(label).querySelector(".menu-item-icon");
const show = (items: NativeContextMenuItem[]) => showJsContextMenu(items, { x: 0, y: 0 }, null);
const item = (label: string, extra: Partial<NativeContextMenuItem> = {}): NativeContextMenuItem => ({ id: label, label, ...extra });

describe("JS context menu icons", () => {
    it("draws a row's icon first, in the shared slot, hidden from assistive tech", () => {
        show([item("Copy", { icon: "copy" })]);
        const icon = slot("Copy")!;
        expect(icon.tagName).toBe("I");
        expect(icon.classList.contains("fa-copy")).toBe(true);
        expect(icon.classList.contains("fa-fw")).toBe(true);
        expect(icon.getAttribute("aria-hidden")).toBe("true");
        expect(row("Copy").firstElementChild).toBe(icon);
        expect(row("Copy").textContent).toBe("Copy");
    });

    it("takes the regular style from a regular@ name", () => {
        show([item("Magnify", { icon: "regular@window-maximize" })]);
        expect(slot("Magnify")!.classList.contains("fa-regular")).toBe(true);
    });

    it("reserves the slot on every row once any row has an icon, so the labels line up", () => {
        show([item("Copy", { icon: "copy" }), item("Open Clipboard URL"), item("", { type: "separator" }), item("Close", { icon: "xmark" })]);
        const bare = slot("Open Clipboard URL")!;
        expect(bare).not.toBeNull();
        expect(bare.classList.contains("fa-copy")).toBe(false);
        expect(bare.getAttribute("aria-hidden")).toBe("true");
        expect(document.querySelectorAll(".menu-divider .menu-item-icon").length).toBe(0);
    });

    it("draws a menu with no icon as before: no slot at all", () => {
        show([item("One"), item("Two")]);
        expect(document.querySelectorAll(".menu-item-icon").length).toBe(0);
    });

    it("keeps a checkable row's check in the slot, and gives it no icon", () => {
        show([item("Word Wrap", { type: "checkbox", checked: true, icon: "copy" }), item("Copy", { icon: "copy" })]);
        const check = slot("Word Wrap")!;
        expect(check.classList.contains("fa-check")).toBe(true);
        expect(check.classList.contains("fa-copy")).toBe(false);
        expect(row("Word Wrap").querySelectorAll(".menu-item-icon").length).toBe(1);
    });

    it("leaves a swatch row's swatch as its leading mark", () => {
        show([item("Default", { icon: "palette" }), item("Blue", { swatchColor: "#3366ff" })]);
        expect(row("Blue").querySelector(".menu-item-swatch")).not.toBeNull();
        expect(row("Blue").querySelector(".menu-item-icon")).toBeNull();
    });

    it("decides the slot per menu level: a submenu of bare rows has none", () => {
        show([item("Group", { type: "submenu", icon: "folder-open", submenu: [item("One"), item("Two")] })]);
        const sub = document.querySelector(".sub-menu")!;
        expect(sub.querySelectorAll(".menu-item-icon").length).toBe(0);
        expect(slot("Group")!.classList.contains("fa-folder-open")).toBe(true);
    });

    it("draws the empty slot, not a broken glyph, for a name it doesn't recognise", () => {
        show([item("Odd", { icon: "Not A Name!" }), item("Copy", { icon: "copy" })]);
        const odd = slot("Odd")!;
        expect(odd.className).toBe("menu-item-icon");
    });

    it("skips a hidden row when deciding whether to reserve the slot", () => {
        show([item("Hidden", { icon: "copy", visible: false }), item("Shown")]);
        expect(document.querySelectorAll(".menu-item-icon").length).toBe(0);
    });
});
