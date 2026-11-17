// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a panel opened from a pane with a backdrop looks like, measured in a real browser
 * (docs/specs/SPEC_PANE_BACKGROUND_THEMES_2026_11_15.md §7.4): a menu and a tooltip that carry the pane's scope
 * (overlay-scope.ts) are drawn with the pane's ink on its overlay surface, a step above the pane and never the same
 * colour as it, for every built-in backdrop on both sides. So are a dialog about the pane, which is mounted inside
 * it, and a toast about it.
 *
 * It compiles the real theme, the pane scope's stylesheet and the menu's, and gives a panel exactly the properties
 * `carryPaneScope` gives it. jsdom cannot resolve `var()` through a stylesheet, so this is a browser test; it skips,
 * rather than fails, on a machine with no usable browser (layout-browser.ts findBrowser).
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import * as sass from "sass";
import { beforeAll, describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "../view/agent/components/layout-browser";
import { BUILTINS, deriveBuiltin, type AppScheme } from "./builtins";
import { overlayScopeVars, scopeVars, type ScopeVars } from "./scope";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const APP = join(__dirname, "..");
const compile = (file: string) => sass.compile(join(APP, file)).css;

/** Tailwind's colour tokens as it puts them on `:root` (the `@theme` block), and the utilities the title tooltip is drawn with. */
function tailwind(): string {
    const setup = readFileSync(join(APP, "../tailwindsetup.css"), "utf8");
    const tokens = [...setup.matchAll(/^\s*(--color-[\w-]+):\s*([^;]+);/gm)].map((m) => `${m[1]}:${m[2]}`).join(";");
    return `:root{${tokens}}
.bg-modalbg{background-color:var(--color-modalbg)}
.border{border-style:solid;border-width:1px}
.border-border{border-color:var(--color-border)}
.text-foreground{color:var(--color-foreground)}`;
}

// An opaque window, as the app sets it for a window that is not see-through (the stylesheet's own value is for one that is).
const OPAQUE_WINDOW = ":root{--window-opacity:1}";

const CSS = [compile("theme.scss"), OPAQUE_WINDOW, tailwind(), compile("backdrop/backdrop.scss"), compile("element/flyoutmenu.scss"), compile("element/ui/menu.scss"), compile("element/modal.scss"), compile("notification/notificationitem.scss")].join("\n");

const inline = (vars: ScopeVars) =>
    Object.entries(vars)
        .map(([name, value]) => `${name}:${value}`)
        .join(";");

const PANEL = `<div class="menu ui-menu" data-menu style="position:static">
  <div class="menu-item ui-menu-item" data-row><span class="label">Copy</span><span class="menu-item-sublabel" data-sub>Ctrl+C</span></div>
  <div class="menu-divider" data-divider></div>
</div>
<div class="bg-modalbg border border-border text-foreground" data-tip>Close the pane</div>
<div class="notification-bubble" data-toast style="position:static"><div class="notification-inner"><div class="notification-text">
  <div class="notification-title green" data-toast-title>Stopped</div><div class="notification-message">The agent has stopped</div>
</div></div></div>`;

// A dialog about a pane: its root is inside the pane (the pane's own modal layer) and carries the scope a panel carries.
const DIALOG = `<div class="modal-panel" data-size="sm" data-dialog style="animation:none">
  <div class="modal-panel-header"><h2 class="modal-panel-title" data-dialog-title>Close the pane?</h2><p class="modal-panel-description" data-dialog-text>Its agent keeps running.</p></div>
  <div class="modal-panel-footer" data-dialog-footer></div>
</div>`;

/** A pane with its scope, and beside it (not inside it, as on `document.body`) a panel with the scope a panel carries. */
function block(id: string, vars: ScopeVars | undefined): string {
    const pane = vars ? `data-backdrop style="${inline(vars)}"` : "";
    const panel = vars ? `data-backdrop style="${inline(overlayScopeVars(vars))}"` : "";
    return `<section data-case="${id}">
<div ${pane}><div data-pane style="background:var(--main-bg-color);color:var(--main-text-color)">pane</div>
<div ${vars ? "data-backdrop" : ""} class="modal-root" data-scope="pane" style="position:static;${vars ? inline(overlayScopeVars(vars)) : ""}">${DIALOG}</div></div>
<div ${panel}>${PANEL}</div>
</section>`;
}

const cases: { id: string; builtin: string; scheme: AppScheme }[] = [];
for (const b of BUILTINS) for (const scheme of ["dark", "light"] as const) cases.push({ id: `${b.id}-${scheme}`, builtin: b.id, scheme });

type Rgba = [number, number, number, number];
interface Measured {
    pane: Rgba;
    menu: Rgba;
    menuBorder: Rgba;
    row: Rgba;
    sub: Rgba;
    divider: Rgba;
    tip: Rgba;
    tipText: Rgba;
    tipBorder: Rgba;
    dialog: Rgba;
    dialogTitle: Rgba;
    dialogText: Rgba;
    dialogBorder: Rgba;
    dialogFooter: Rgba;
    toast: Rgba;
    toastText: Rgba;
    toastTitle: Rgba;
    toastBorder: Rgba;
}

// Colours are read back through a canvas pixel, so every form a computed colour takes (`rgb()`, `color(srgb …)` out
// of a color-mix) comes out as the same four bytes.
const MEASURE = `
const cv = document.createElement("canvas"); cv.width = cv.height = 1;
const ctx = cv.getContext("2d", { willReadFrequently: true });
const rgba = (c) => { ctx.clearRect(0, 0, 1, 1); ctx.fillStyle = c; ctx.fillRect(0, 0, 1, 1); return Array.from(ctx.getImageData(0, 0, 1, 1).data); };
const out = {};
for (const s of document.querySelectorAll("[data-case]")) {
  const cs = (sel) => getComputedStyle(s.querySelector(sel));
  out[s.dataset.case] = {
    pane: rgba(cs("[data-pane]").backgroundColor),
    menu: rgba(cs("[data-menu]").backgroundColor),
    menuBorder: rgba(cs("[data-menu]").borderTopColor),
    row: rgba(cs("[data-row]").color),
    sub: rgba(cs("[data-sub]").color),
    divider: rgba(cs("[data-divider]").backgroundColor),
    tip: rgba(cs("[data-tip]").backgroundColor),
    tipText: rgba(cs("[data-tip]").color),
    tipBorder: rgba(cs("[data-tip]").borderTopColor),
    dialog: rgba(cs("[data-dialog]").backgroundColor),
    dialogTitle: rgba(cs("[data-dialog-title]").color),
    dialogText: rgba(cs("[data-dialog-text]").color),
    dialogBorder: rgba(cs("[data-dialog]").borderTopColor),
    dialogFooter: rgba(cs("[data-dialog-footer]").backgroundColor),
    toast: rgba(cs("[data-toast]").backgroundColor),
    toastText: rgba(cs("[data-toast]").color),
    toastTitle: rgba(cs("[data-toast-title]").color),
    toastBorder: rgba(cs("[data-toast]").borderTopColor),
  };
}
const pre = document.createElement("pre"); pre.id = "result"; pre.textContent = JSON.stringify(out); document.body.appendChild(pre);
`;

/** `#rrggbb` or `#rrggbbaa` as four bytes. */
function bytes(hex: string): Rgba {
    const h = hex.replace("#", "");
    const n = (i: number) => parseInt(h.slice(i, i + 2), 16);
    return [n(0), n(2), n(4), h.length >= 8 ? n(6) : 255];
}

/** The same colour: to a byte when opaque; a see-through colour loses a little in the canvas's own arithmetic. */
function expectColour(actual: Rgba, hex: string, what: string): void {
    const want = bytes(hex);
    const slack = want[3] === 255 ? 1 : 12;
    for (let i = 0; i < 3; i++) expect(Math.abs(actual[i] - want[i]), `${what}: ${actual} against ${hex}`).toBeLessThanOrEqual(slack);
    expect(Math.abs(actual[3] - want[3]), `${what}: alpha of ${actual} against ${hex}`).toBeLessThanOrEqual(2);
}

/**
 * A see-through wash of a colour. The canvas stores a colour multiplied by its alpha, so a faint wash comes back in
 * coarse steps (at 3%, steps of about 32): the slack is one such step.
 */
function expectTintOf(actual: Rgba, hex: string, what: string): void {
    expect(actual[3], `${what} is see-through`).toBeLessThan(255);
    expect(actual[3], `${what} is there`).toBeGreaterThan(0);
    const want = bytes(hex);
    const slack = Math.ceil(255 / actual[3]) + 2;
    for (let i = 0; i < 3; i++) expect(Math.abs(actual[i] - want[i]), `${what} ${actual} is a tint of ${hex}`).toBeLessThanOrEqual(slack);
}

const differ = (a: Rgba, b: Rgba) => a.some((v, i) => Math.abs(v - b[i]) > 1);

describe.skipIf(!BROWSER)("a panel opened from a pane with a backdrop", () => {
    let r: Record<string, Measured>;
    beforeAll(async () => {
        const body = [block("window", undefined), ...cases.map((c) => block(c.id, scopeVars(deriveBuiltin(c.builtin, c.scheme)!)))].join("");
        const html = `<!doctype html><meta charset="utf-8"><style>${CSS}</style>${body}<script>window.addEventListener("load",()=>{${MEASURE}})</script>`;
        r = await measureInBrowser<Record<string, Measured>>(BROWSER!, html, { width: 900, height: 900 });
    }, 60_000);

    it("with no scope, is drawn in the window's theme (the page reproduces the app's own colours)", () => {
        const m = r.window;
        // The default theme's tooltip surface, from Tailwind's own token.
        expectColour(m.tip, "#232323", "the tooltip's surface");
        expect(m.menu[3], "the menu has a surface").toBe(255);
        expect(m.row[3], "the menu's text has a colour").toBe(255);
    });

    it.each(cases)("$id: a menu is on the pane's overlay surface, in the pane's ink", ({ id, builtin, scheme }) => {
        const d = deriveBuiltin(builtin, scheme)!;
        const m = r[id];
        expectColour(m.pane, d.surfaces.surface, "the pane's surface");
        expectColour(m.menu, d.surfaces.overlay, "the menu's surface");
        expect(differ(m.menu, m.pane), "the menu is not the colour of the pane it lies over").toBe(true);
        expectColour(m.row, d.ink.text, "a row's text");
        expectColour(m.sub, d.ink.muted, "a row's second line");
        expectColour(m.menuBorder, d.ink.border, "the menu's border");
    });

    it.each(cases)("$id: a colour built on a token is built on the pane's (the menu's divider)", ({ id, builtin, scheme }) => {
        const d = deriveBuiltin(builtin, scheme)!;
        const m = r[id];
        // The divider is a tint of the text colour: the pane's text, not the window's.
        expectTintOf(m.divider, d.ink.text, "the divider");
    });

    it.each(cases)("$id: a tooltip drawn with Tailwind's classes follows the pane too", ({ id, builtin, scheme }) => {
        const d = deriveBuiltin(builtin, scheme)!;
        const m = r[id];
        expectColour(m.tip, d.surfaces.overlay, "the tooltip's surface");
        expectColour(m.tipText, d.ink.text, "the tooltip's text");
        expectColour(m.tipBorder, d.ink.border, "the tooltip's border");
    });

    it.each(cases)("$id: a dialog about the pane is on its overlay surface, in its ink", ({ id, builtin, scheme }) => {
        const d = deriveBuiltin(builtin, scheme)!;
        const m = r[id];
        expectColour(m.dialog, d.surfaces.overlay, "the dialog's surface");
        expect(differ(m.dialog, m.pane), "the dialog is not the colour of the pane it covers").toBe(true);
        expectColour(m.dialogTitle, d.ink.text, "the dialog's title");
        expectColour(m.dialogText, d.ink.muted, "the dialog's description");
        expectColour(m.dialogBorder, d.ink.border, "the dialog's border");
        // The footer's wash is a tint of the text colour: the pane's text.
        expectTintOf(m.dialogFooter, d.ink.text, "the footer");
    });

    it.each(cases)("$id: a toast about the pane is on its overlay surface, in its ink", ({ id, builtin, scheme }) => {
        const d = deriveBuiltin(builtin, scheme)!;
        const m = r[id];
        expectColour(m.toast, d.surfaces.overlay, "the toast's surface");
        expectColour(m.toastText, d.ink.text, "the toast's text");
        expectColour(m.toastTitle, d.ink.success, "the toast's title");
        expectTintOf(m.toastBorder, d.ink.text, "the toast's border");
    });
});
