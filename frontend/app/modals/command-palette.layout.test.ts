// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The command palette's place and size, measured in a real browser (SPEC_COMMAND_PALETTE_TOP_CENTRE_2026_11_17.md
 * R1 to R5, R12, R14): it hangs 6 px below the window's top edge, centred, 600 px wide at 100% zoom and never wider
 * than the window less 8 px each side, at every chrome zoom. It lists at most sixteen rows and then scrolls, with the
 * search row staying put, and never grows past the window's height less 48 px. Nothing behind it is dimmed. A label
 * too long for its row is cut, and the shortcut and the group's name stay whole.
 *
 * This compiles the real `modal.scss`, `command-palette.scss` and the kit's styles and lays the panel out the way
 * `command-palette.tsx` and `palette-row.tsx` do. It skips, rather than fails, on a machine with no usable browser
 * (layout-browser.ts findBrowser).
 */

import { join } from "node:path";
import * as sass from "sass";
import { describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "@/app/view/agent/components/layout-browser";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const css = [
    join(__dirname, "../element/modal.scss"),
    join(__dirname, "../element/ui/ui.scss"),
    join(__dirname, "command-palette.scss"),
]
    .map((f) => sass.compile(f).css)
    .join("\n");

const tokens = (zoom: number) => `:root{--zoomfactor:${zoom};--z-modal:1000;--main-text-color:#ddd;--secondary-text-color:#999;
--border-color:#444;--main-bg-color:#222;--block-bg-color:#2a2a2a;--hover-bg-color:#333;--highlight-bg-color:#444;
--font-sans:Arial,sans-serif;--font-mono:monospace;--text-2xs:10px;--text-xs:11px;--text-sm:12px;--text-base:13px;--text-md:14px;
--space-1:4px;--space-2:8px;--space-3:12px;--space-4:16px;--space-6:24px;--radius-sm:3px;--radius-lg:8px;
--shadow-modal:0 4px 16px #000;--duration-quick:0ms;--scrollbar-thumb-color:#555}
*,*::before,*::after{box-sizing:border-box}
body{margin:0;background:#111;color:#ddd;font:13px Arial,sans-serif}`;

function row(label: string, keys: string[], group = "", then: string[] = []): string {
    const step = (ks: string[]) =>
        `<span class="command-palette-item-step">${ks.map((k) => `<kbd class="ui-kbd">${k}</kbd>`).join("")}</span>`;
    const caps = keys.length ? `<span class="command-palette-item-shortcut">${step(keys)}${then.length ? step(then) : ""}</span>` : "";
    return `<div class="command-palette-item"><span class="command-palette-item-text"><span class="command-palette-item-label">${label}</span></span>${caps}<span class="command-palette-item-group">${group}</span></div>`;
}

function page(zoom: number, rows: string): string {
    return (
        `<!doctype html><meta charset="utf-8"><style>${tokens(zoom)}\n${css}</style>` +
        `<div class="modal-root" data-scope="window" data-placement="dropdown" data-backdrop="none">` +
        `<div class="modal-backdrop"></div>` +
        `<div class="modal-panel command-palette-panel"><div class="command-palette-container">` +
        `<div class="command-palette-input-row"><input class="command-palette-input" value="">` +
        `</div><div class="command-palette-list">${rows}</div></div></div></div>` +
        `<script>window.addEventListener("load",()=>{${MEASURE}})</script>`
    );
}

const MEASURE = `
const r = (el) => el.getBoundingClientRect();
const panel = r(document.querySelector(".modal-panel"));
const list = document.querySelector(".command-palette-list");
list.scrollTop = 500;
// What a key move does to the selected row: a script's scroll must reach the list, never the panel around it.
const all = document.querySelectorAll(".command-palette-item");
all[all.length - 1].scrollIntoView({ block: "nearest" });
const input = r(document.querySelector(".command-palette-input-row"));
const vw = document.documentElement.clientWidth, vh = document.documentElement.clientHeight;
const caps = [...document.querySelectorAll(".command-palette-item-shortcut")].map((el) => r(el));
const labels = [...document.querySelectorAll(".command-palette-item-label")].map((el) => r(el).width);
const groups = [...document.querySelectorAll(".command-palette-item-group")].map((el) => r(el));
const backdrop = getComputedStyle(document.querySelector(".modal-backdrop"));
const out = {
  vw, vh, top: panel.top, left: panel.left, right: vw - panel.right, width: panel.width, height: panel.height, bottom: panel.bottom,
  inputTop: input.top - panel.top, listClient: list.clientHeight, listScroll: list.scrollHeight, scrolled: list.scrollTop > 0,
  capsWidths: caps.map((c) => c.width), capsRights: caps.map((c) => c.right), panelRight: panel.right,
  groupRights: groups.map((g) => g.right), groupWidths: groups.map((g) => g.width),
  backdropBg: backdrop.backgroundColor, backdropFilter: backdrop.backdropFilter,
  panelScroll: document.querySelector(".modal-panel").scrollTop, labelWidths: labels, listScrollW: list.scrollWidth, listClientW: list.clientWidth,
  groupShown: [...document.querySelectorAll(".command-palette-item-group")].some((g) => getComputedStyle(g).display !== "none"),
};
const pre = document.createElement("pre"); pre.id = "result"; pre.textContent = JSON.stringify(out); document.body.appendChild(pre);
`;

type Measured = Record<string, any>;
const few = [row("New Tab", ["Ctrl", "T"], "tab"), row("Close Tab", []), row("Close Pane", [], "pane")].join("");
const many = Array.from({ length: 80 }, (_, i) => row(`Command ${i}`, i % 2 ? ["Ctrl", "Shift", "P"] : [], i % 10 ? "" : "open")).join("");

describe.skipIf(!BROWSER)("command palette place and size", () => {
    const cases: [number, number][] = [
        [400, 1],
        [600, 1],
        [900, 1],
        [1600, 1],
        [600, 0.8],
        [600, 1.5],
        [1600, 1.5],
        [600, 2],
        [1600, 2],
    ];
    for (const [width, zoom] of cases) {
        it(`hangs from 6px below the top, centred, within the window, at ${width}px wide and ${zoom * 100}% zoom`, async () => {
            const m = await measureInBrowser<Measured>(BROWSER!, page(zoom, many), { width, height: 700 });
            expect(m.top).toBe(6);
            expect(Math.abs(m.left - m.right), JSON.stringify(m)).toBeLessThanOrEqual(1);
            expect(m.width).toBeLessThanOrEqual(m.vw - 16 + 1);
            expect(m.left).toBeGreaterThanOrEqual(8 - 1);
            if (m.vw - 16 >= 600 * zoom) expect(Math.abs(m.width - 600 * zoom)).toBeLessThanOrEqual(1);
            else expect(Math.abs(m.width - (m.vw - 16))).toBeLessThanOrEqual(1);
        }, 60_000);
    }

    it("is as tall as its rows need when there are few, and scrolls its list past sixteen rows with the search row in place", async () => {
        const fewM = await measureInBrowser<Measured>(BROWSER!, page(1, few), { width: 900, height: 700 });
        expect(fewM.listScroll).toBeLessThanOrEqual(fewM.listClient + 1);
        const manyM = await measureInBrowser<Measured>(BROWSER!, page(1, many), { width: 900, height: 700 });
        expect(manyM.listScroll).toBeGreaterThan(manyM.listClient);
        expect(manyM.listClient).toBeLessThanOrEqual(16 * 24 + 8 + 1);
        expect(manyM.scrolled).toBe(true);
        expect(manyM.inputTop).toBeLessThanOrEqual(1); // the panel's own border
        expect(manyM.height).toBeGreaterThan(fewM.height);
    }, 60_000);

    it("never grows past the window's height less 48px", async () => {
        const m = await measureInBrowser<Measured>(BROWSER!, page(1, many), { width: 900, height: 300 });
        expect(m.height).toBeLessThanOrEqual(300 - 48 + 1);
        expect(m.inputTop).toBeLessThanOrEqual(1);
    }, 60_000);

    it("keeps every label some room, and scrolls nothing sideways, in a narrow panel with four-key shortcuts", async () => {
        const rows = [
            row("Split Down", ["Ctrl", "Alt", "Shift", "D"], "split"),
            row("Open Agent", ["Ctrl", "Shift", "A"], "open"),
            row("New Pane", ["Ctrl", "Shift", "N"], "window"),
            // A chord: two steps, seven keys.
            row("Swap Pane Down", ["Ctrl", "Shift", "S"], "pane", ["Shift", "\u2193"]),
            row("Move Tab Right", ["Ctrl", "Shift", "T"], "tab", ["Shift", "\u2192"]),
        ].join("");
        const m = await measureInBrowser<Measured>(BROWSER!, page(1.5, rows), { width: 400, height: 700 });
        expect(Math.min(...m.labelWidths)).toBeGreaterThanOrEqual(40);
        expect(m.listScrollW).toBeLessThanOrEqual(m.listClientW);
        // No shortcut is cut by the right edge of its row.
        expect(m.capsRights.every((r: number) => r <= m.panelRight)).toBe(true);
        // The panel is narrower than 440 css px here: the group's name is left out.
        expect(m.groupShown).toBe(false);
    }, 60_000);

    it("keeps the group's name where the panel is wide enough for it", async () => {
        const m = await measureInBrowser<Measured>(BROWSER!, page(1, row("Open Agent", ["Ctrl", "Shift", "A"], "open")), {
            width: 900,
            height: 700,
        });
        expect(m.groupShown).toBe(true);
    }, 60_000);

    it("never scrolls the panel itself, in a window shorter than the panel", async () => {
        const m = await measureInBrowser<Measured>(BROWSER!, page(1, many), { width: 900, height: 300 });
        expect(m.panelScroll).toBe(0);
        expect(m.inputTop).toBeLessThanOrEqual(1);
    }, 60_000);

    it("dims and blurs nothing behind it", async () => {
        const m = await measureInBrowser<Measured>(BROWSER!, page(1, few), { width: 900, height: 700 });
        expect(m.backdropBg).toBe("rgba(0, 0, 0, 0)");
        expect(m.backdropFilter).toBe("none");
    }, 60_000);

    it("keeps the shortcut and the group name whole when a label is far too long", async () => {
        const long = "A very long command name ".repeat(8);
        const rows = [row("Short", ["Ctrl", "Shift", "P"], "open"), row(long, ["Ctrl", "Shift", "P"], "open")].join("");
        const m = await measureInBrowser<Measured>(BROWSER!, page(1, rows), { width: 900, height: 700 });
        expect(m.capsWidths[1]).toBeCloseTo(m.capsWidths[0], 0);
        expect(m.groupWidths[1]).toBeCloseTo(m.groupWidths[0], 0);
        expect(m.capsRights[1]).toBeLessThanOrEqual(m.panelRight);
        expect(m.capsRights[1]).toBeCloseTo(m.capsRights[0], 0);
    }, 60_000);
});
