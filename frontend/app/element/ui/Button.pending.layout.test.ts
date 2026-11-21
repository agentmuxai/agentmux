// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A button's box does not change when its action goes pending, measured in a real browser
 * (docs/specs/SPEC_BUTTON_PRESSED_AND_PENDING_FEEDBACK_2026_11_20.md §4, §9): the same width and height at rest, while
 * pending before the spinner, and with the spinner over the label, for each variant, at three chrome zooms. Skips
 * with no usable browser.
 *
 * The markup is what `Button` renders in each state (Button.pending.test.tsx holds the component to it); the point
 * here is the stylesheet: a spinner over a label must take no room, and the label it hides must keep its own.
 */

import { join } from "node:path";
import * as sass from "sass";
import { describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser } from "@/app/view/agent/components/layout-browser";

const BROWSER = findBrowser();
const css = sass.compile(join(__dirname, "ui.scss")).css;
const TOKENS = `:root{--accent-color:#58c142;--secondary-text-color:#aaa;--main-text-color:#eee;--border-color:#555;--highlight-bg-color:#333;
--ui-height:28px;--ui-pad-x:12px;--ui-pad-y:4px;--ui-gap:6px;--ui-font-size:13px;--ui-icon-size:12px;--ui-font-weight:500;--leading-tight:1.2;--radius-sm:2px}
body{margin:0;font:13px sans-serif}.row{display:flex;gap:8px;align-items:flex-start;padding:4px}
.glyph{display:inline-block;width:1em;height:1em}`;

const GLYPH = '<i class="glyph"></i>';
const SPINNER = '<span class="ui-button-spinner" aria-hidden="true"><i class="glyph fa-spin"></i></span>';

type State = "rest" | "pending" | "spinning";

interface Variant {
    id: string;
    /** The button's classes. */
    cls: string;
    /** Its children. */
    inner: string;
    /** Whether the spinner takes the icon's place (true) or sits over the label (false). */
    hasIcon: boolean;
}

const VARIANTS: Variant[] = [
    { id: "label", cls: "ui-button ui-tone-neutral", inner: "Save changes", hasIcon: false },
    { id: "accent", cls: "ui-button ui-tone-accent", inner: "Install", hasIcon: false },
    { id: "icon-and-label", cls: "ui-button ui-tone-neutral", inner: `${GLYPH}Refresh`, hasIcon: true },
    { id: "icon-only", cls: "ui-button ui-tone-quiet ui-icon-button", inner: GLYPH, hasIcon: true },
    { id: "row", cls: "ui-button ui-tone-quiet ui-button--row", inner: "Open the project folder", hasIcon: false },
    { id: "link", cls: "ui-button ui-tone-quiet ui-button--link", inner: "Show details", hasIcon: false },
    { id: "chrome", cls: "ui-button--chrome", inner: GLYPH, hasIcon: true },
];

function markup(v: Variant, state: State): string {
    const pending = state !== "rest";
    // The icon's place is taken by the spinner glyph itself; a label-only button gets the overlay and the class.
    const over = state === "spinning" && !v.hasIcon;
    const inner = state === "spinning" && v.hasIcon ? v.inner.replace(GLYPH, '<i class="glyph fa-spin"></i>') : v.inner;
    const cls = `${v.cls}${over ? " ui-button--spin-over" : ""}`;
    return `<button type="button" data-case="${v.id}-${state}" class="${cls}"${pending ? ' aria-busy="true" data-pending="true"' : ""}>${inner}${over ? SPINNER : ""}</button>`;
}

const ZOOMS = [0.5, 1, 1.5];
const STATES: State[] = ["rest", "pending", "spinning"];
const body = ZOOMS.map((z) => `<div class="row" style="zoom:${z}" data-zoom="${z}">${VARIANTS.flatMap((v) => STATES.map((s) => markup(v, s))).join("")}</div>`).join("");

const MEASURE = `
const out = {};
for (const row of document.querySelectorAll("[data-zoom]")) {
  for (const el of row.querySelectorAll("[data-case]")) {
    const r = el.getBoundingClientRect(), cs = getComputedStyle(el);
    const sp = el.querySelector(".ui-button-spinner");
    out[row.dataset.zoom + "/" + el.dataset.case] = {
      w: Math.round(r.width * 100) / 100, h: Math.round(r.height * 100) / 100,
      fill: cs.webkitTextFillColor, color: cs.color,
      spinnerFill: sp ? getComputedStyle(sp).webkitTextFillColor : null,
      spinnerBox: sp ? (() => { const s = sp.getBoundingClientRect(); return { w: s.width, h: s.height }; })() : null,
    };
  }
}
const pre = document.createElement("pre"); pre.id = "result"; pre.textContent = JSON.stringify(out); document.body.appendChild(pre);
`;

interface Seen {
    w: number;
    h: number;
    fill: string;
    color: string;
    spinnerFill: string | null;
    spinnerBox: { w: number; h: number } | null;
}

describe.skipIf(!BROWSER)("a pending button", () => {
    it("keeps its box through rest, pending and the spinner, in every variant and zoom", async () => {
        const html = `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>${body}<script>window.addEventListener("load",()=>{${MEASURE}})</script>`;
        const r = await measureInBrowser<Record<string, Seen>>(BROWSER!, html, { width: 1400, height: 400 });
        for (const z of ZOOMS) {
            for (const v of VARIANTS) {
                const rest = r[`${z}/${v.id}-rest`];
                for (const s of ["pending", "spinning"] as const) {
                    const m = r[`${z}/${v.id}-${s}`];
                    expect(m.w, `${v.id} at ${z}: width while ${s}`).toBeCloseTo(rest.w, 1);
                    expect(m.h, `${v.id} at ${z}: height while ${s}`).toBeCloseTo(rest.h, 1);
                }
            }
        }
    }, 60_000);

    it("hides the label of a label-only button with the text fill, and draws the spinner in the button's own colour", async () => {
        const html = `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>${body}<script>window.addEventListener("load",()=>{${MEASURE}})</script>`;
        const r = await measureInBrowser<Record<string, Seen>>(BROWSER!, html, { width: 1400, height: 400 });
        for (const id of ["label", "accent", "row", "link"]) {
            const m = r[`1/${id}-spinning`];
            expect(m.fill, `${id}: the label is not drawn`).toBe("rgba(0, 0, 0, 0)");
            expect(m.spinnerFill, `${id}: the spinner is`).toBe(m.color);
            expect(m.spinnerBox!.w, `${id}: the spinner has room over the label`).toBeGreaterThan(0);
        }
        // Not hidden before the spinner, nor at rest.
        expect(r["1/label-pending"].fill).toBe(r["1/label-pending"].color);
        expect(r["1/label-rest"].fill).toBe(r["1/label-rest"].color);
    }, 60_000);
});

describe("the press", () => {
    const rule = (selector: RegExp) => css.split("}").find((block) => selector.test(block.split("{")[0])) ?? "";

    it("shows in the frame of the mouse-down: a pressed look with no transition, for every tone but the filled one", () => {
        const press = rule(/^\s*\.ui-button:active:not\(/);
        expect(press, "a :active rule for the kit's button").not.toBe("");
        expect(press).toContain("transition-duration: 0s");
        expect(press).toContain("var(--accent-color)");
        expect(rule(/\.ui-button\.ui-tone-attention:active/)).toContain("color-mix");
    });

    it("drops a chrome icon to the disabled step at once", () => {
        const press = rule(/\.ui-button--chrome:active/);
        expect(press).toContain("opacity: 0.45");
        expect(press).toContain("transition-duration: 0s");
    });
});
