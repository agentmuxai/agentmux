// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A menu, a submenu, a popover, a dialog and a drop-down are fully solid while the window is faded (#48092),
 * measured in a real browser.
 *
 * The window's opacity reaches the page two ways, and a floating surface must come out opaque through both:
 *  - the window opacity on the page (`--window-opacity`, as the app sets it on the root): the pane's own
 *    backgrounds go see-through with it, a floating surface's must not;
 *  - the page fade (`am-uniform-fade`, which the host turns on when the page, not the OS, carries the fade): the
 *    app root and the page background fade, and what a portal puts outside the app root (a menu, a popover) is
 *    outside that group, so nothing multiplies its opacity.
 * A see-through pane is measured in both as the control: a test that cannot see a fade proves nothing.
 *
 * It compiles the real theme and the surfaces' own stylesheets. jsdom cannot resolve `var()` through a
 * stylesheet, so this is a browser test; it skips, rather than fails, with no usable browser (layout-browser.ts).
 */

import { join } from "node:path";
import * as sass from "sass";
import { beforeAll, describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "../view/agent/components/layout-browser";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const APP = join(__dirname, "..");
const compile = (file: string) => sass.compile(join(APP, file)).css;
const POPOVER = sass.compileString(
    '@use "element/popover-frame" as p; .t-popover { @include p.popover-frame; }',
    { loadPaths: [APP] }
).css;

const CSS = [
    compile("theme.scss"),
    compile("element/flyoutmenu.scss"),
    compile("element/ui/menu.scss"),
    compile("element/modal.scss"),
    POPOVER,
].join("\n");

const SURFACES = `
<div class="menu ui-menu" data-menu style="position:static"><div class="menu-item ui-menu-item"><span class="label">Copy</span></div></div>
<div class="menu sub-menu ui-menu" data-submenu style="position:static"><div class="menu-item ui-menu-item"><span class="label">Dark</span></div></div>
<div class="t-popover" data-popover>CPU 12%</div>
<div class="modal-panel" data-size="sm" data-dialog style="animation:none"><div class="modal-panel-header"><h2 class="modal-panel-title">Close?</h2></div></div>
<select data-select><option data-option>One</option></select>`;
const NAMES = ["menu", "submenu", "popover", "dialog", "option"] as const;
type Name = (typeof NAMES)[number] | "pane";

interface Seen {
    /** The background's own alpha, 0 to 255. */
    alpha: number;
    /** The opacity of the element and all of its ancestors multiplied: how much of it reaches the screen. */
    eff: number;
    /** The element's own `opacity` (the page background only). */
    own?: number;
}
type Measured = Record<Name | "pageBackground" | "html" | "body", Seen>;

const MEASURE = `
const cv = document.createElement("canvas"); cv.width = cv.height = 1;
const ctx = cv.getContext("2d", { willReadFrequently: true });
const alphaOf = (c) => { ctx.clearRect(0, 0, 1, 1); ctx.fillStyle = c; ctx.fillRect(0, 0, 1, 1); return ctx.getImageData(0, 0, 1, 1).data[3]; };
const eff = (el) => { let o = 1; for (let n = el; n; n = n.parentElement) o *= parseFloat(getComputedStyle(n).opacity); return o; };
const seen = (el) => ({ alpha: alphaOf(getComputedStyle(el).backgroundColor), eff: eff(el) });
const q = (s) => document.querySelector(s);
const out = {
  pane: seen(q("[data-pane]")), menu: seen(q("[data-menu]")), submenu: seen(q("[data-submenu]")), popover: seen(q("[data-popover]")),
  dialog: seen(q("[data-dialog]")), option: seen(q("[data-option]")), html: seen(document.documentElement), body: seen(document.body),
};
const before = getComputedStyle(q("#main"), "::before");
out.pageBackground = { alpha: alphaOf(before.backgroundColor), eff: parseFloat(before.opacity) * eff(q("#main")), own: parseFloat(before.opacity) };
const pre = document.createElement("pre"); pre.id = "result"; pre.textContent = JSON.stringify(out); document.body.appendChild(pre);
`;

/** One page at a window opacity: the root as the app sets it, with the page fade on or off. */
function page(opacity: number, pageFade: boolean): string {
    const root = pageFade
        ? `class="am-uniform-fade" style="--am-window-fade:${opacity}; --window-opacity:${opacity}"`
        : `style="--window-opacity:${opacity}"`;
    // The app root holds the pane; a portal's surfaces sit beside it under body, as the menus and popovers do.
    const body = `<div id="main"><div data-pane style="background:var(--main-bg-color)">pane</div></div>${SURFACES}`;
    return `<!doctype html><html ${root}><meta charset="utf-8"><style>${CSS}\n*{animation:none!important;transition:none!important}</style><body>${body}<script>window.addEventListener("load",()=>{${MEASURE}})</script></body></html>`;
}

const LEVELS = [0.35, 0.65, 1];

describe.skipIf(!BROWSER)("floating surfaces under a faded window", () => {
    const withValue: Record<string, Measured> = {};
    const withPage: Record<string, Measured> = {};
    beforeAll(async () => {
        for (const o of LEVELS) {
            withValue[o] = await measureInBrowser<Measured>(BROWSER!, page(o, false), { width: 700, height: 700 });
            withPage[o] = await measureInBrowser<Measured>(BROWSER!, page(o, true), { width: 700, height: 700 });
        }
    }, 120_000);

    describe.each(LEVELS)("window opacity %s", (o) => {
        it.each(NAMES)("the window opacity on the page: the %s is fully solid", (name) => {
            const m = withValue[o][name];
            expect(m.alpha, `the ${name}'s background alpha`).toBe(255);
            expect(m.eff, `the ${name}'s own opacity`).toBe(1);
        });

        it("the window opacity on the page: the pane does fade (the control)", () => {
            const pane = withValue[o].pane;
            if (o < 1) expect(pane.alpha, "the pane's background is see-through").toBeLessThan(255);
            expect(Math.abs(pane.alpha - Math.round(o * 255)), "by the window opacity").toBeLessThanOrEqual(2);
        });

        it.each(NAMES)("the page fade: the %s is fully solid and nothing fades it", (name) => {
            const m = withPage[o][name];
            expect(m.alpha, `the ${name}'s background alpha`).toBe(255);
            expect(m.eff, `the ${name} reaches the screen at full strength`).toBe(1);
        });

        it("the page fade: the app root carries the fade once, the page background inside it, and the page's own are clear", () => {
            const m = withPage[o];
            expect(m.pane.eff, "the pane's strength on the screen").toBeCloseTo(o, 3);
            expect(m.pageBackground.eff, "the page background's strength").toBeCloseTo(o, 3);
            // One group: a second fade on the background layer would compound with the pane's (at 35% the window came out
            // 57% opaque, at 65% 88%).
            expect(m.pageBackground.own, "the background is not faded on its own").toBe(1);
            expect(m.html.alpha, "html is clear").toBe(0);
            expect(m.body.alpha, "body is clear").toBe(0);
        });
    });
});
