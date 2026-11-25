// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

// jsdom can't lay out a floating panel; only the placing is stubbed.
vi.mock("@/app/util/menu-position", () => ({
    computeMenuPosition: vi.fn(async () => ({
        style: { position: "fixed", left: "0px", top: "0px" },
        placement: "bottom-start",
        maxHeight: Infinity,
        maxWidth: Infinity,
    })),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetConfigCommand: vi.fn(async () => undefined) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const ran = vi.hoisted(() => [] as string[]);
// The registry itself is not loaded: a command's name and action come through this one seam.
vi.mock("@/app/store/command-menu-parts", () => ({
    commandMenuParts: (id: string) => ({ label: `name of ${id}`, shortcut: "", run: () => ran.push(id) }),
}));
vi.mock("@/util/host-caps", () => ({ hostHas: () => true }));

import { TitleBarContextMenu } from "./titlebar-context-menu";

afterEach(() => cleanup());

const widget = (label: string, icon: string, order: number, color?: string) => ({
    label,
    icon,
    color,
    "display:order": order,
    blockdef: { meta: { view: label.toLowerCase() } },
});

const mount = () =>
    render(() => (
        <TitleBarContextMenu
            pos={{ x: 0, y: 0 }}
            onClose={vi.fn()}
            fullConfig={{
                settings: { "widget:pinned": ["agent"] },
                widgets: {
                    "defwidget@agent": widget("Agent", "wand-magic-sparkles", 1, "#f0a020"),
                    "defwidget@files": widget("Files", "folder", 2, "#33aadd"),
                },
            }}
        />
    ));

describe("the window header's right-click menu", () => {
    it("names New Window and New Tab from the command registry and runs them as it does", () => {
        ran.length = 0;
        mount();
        const rows = [...document.body.querySelectorAll<HTMLElement>(".popover-menu-action")];
        const row = (name: string) => rows.find((r) => r.querySelector(".label")?.textContent === name);
        expect(row("New Window")).toBeUndefined();
        row("name of window:new")?.click();
        row("name of tab:new")?.click();
        expect(ran).toEqual(["window:new", "tab:new"]);
    });

    it("no longer offers to open widgets.json", () => {
        mount();
        expect(document.body.textContent).not.toContain("widgets.json");
    });

    it("lists every widget under Pin Widgets with its icon, in the widget's own colour, after its check", () => {
        mount();
        const rows = [...document.body.querySelectorAll<HTMLElement>(".popover-menu-section-items .popover-menu-action")];
        expect(rows.map((r) => r.querySelector(".label")?.textContent)).toEqual(["Agent", "Files"]);
        for (const [row, glyph] of [
            [rows[0], "fa-wand-magic-sparkles"],
            [rows[1], "fa-folder"],
        ] as const) {
            const [check, icon] = [...row.querySelectorAll<HTMLElement>(".menu-item-icon")];
            expect(check.classList.contains("menu-item-check")).toBe(true);
            expect(icon.classList.contains(glyph)).toBe(true);
            expect(icon.style.getPropertyValue("--menu-icon-color")).not.toBe("");
        }
        // The pinned one is checked, the other is not.
        expect(rows[0].querySelector(".fa-check")).not.toBeNull();
        expect(rows[1].querySelector(".fa-check")).toBeNull();
    });
});
