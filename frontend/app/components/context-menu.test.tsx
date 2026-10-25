// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ContextMenu, type ContextMenuItem } from "./context-menu";

afterEach(() => cleanup());

const mount = (items: ContextMenuItem[]) => render(() => <ContextMenu items={items} x={0} y={0} onClose={vi.fn()} />);
const row = (label: string): HTMLElement =>
    [...document.querySelectorAll<HTMLElement>(".ctx-menu-item")].find((r) => r.querySelector(".ctx-menu-label")?.textContent === label)!;
const action = (label: string, extra: Partial<ContextMenuItem> = {}): ContextMenuItem => ({ type: "action", label, ...extra });

describe("ContextMenu icons", () => {
    it("draws the icon before the label, hidden from assistive tech", () => {
        mount([action("Rename", { icon: "pen", shortcut: "F2" })]);
        const icon = row("Rename").querySelector(".menu-item-icon")!;
        expect(icon.classList.contains("fa-pen")).toBe(true);
        expect(icon.getAttribute("aria-hidden")).toBe("true");
        expect(icon.nextElementSibling?.textContent).toBe("Rename");
        expect(row("Rename").querySelector(".ctx-menu-shortcut")?.textContent).toBe("F2");
    });

    it("reserves the slot on every action row once one has an icon", () => {
        mount([action("Open", { icon: "folder-open" }), { type: "separator" }, action("Copy path")]);
        const bare = row("Copy path").querySelector(".menu-item-icon")!;
        expect(bare).not.toBeNull();
        expect(bare.classList.contains("fa-folder-open")).toBe(false);
        expect(document.querySelectorAll(".ctx-menu-sep .menu-item-icon").length).toBe(0);
    });

    it("draws a menu with no icon as before", () => {
        mount([action("Open"), action("Copy path")]);
        expect(document.querySelectorAll(".menu-item-icon").length).toBe(0);
    });

    it("lets a danger row's icon take the row's colour", () => {
        mount([action("Delete", { icon: "trash", danger: true })]);
        const r = row("Delete");
        expect(r.classList.contains("ctx-menu-item--danger")).toBe(true);
        expect(r.querySelector(".fa-trash")).not.toBeNull();
    });
});
