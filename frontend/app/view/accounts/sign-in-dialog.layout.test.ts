// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The sign-in over an agent pane, measured in a real browser: a pane too small for it scrolls inside the pane (the dialog
 * used to clip, so a field taking focus scrolled its top out of sight with no bar), it follows the pane's zoom, and its
 * button rows wrap rather than run off the side. The docked card scrolls the same way. Skips with no usable browser.
 */

import { join } from "node:path";
import * as sass from "sass";
import { describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser } from "@/app/view/agent/components/layout-browser";

const BROWSER = findBrowser();
const compile = (rel: string) => sass.compile(join(__dirname, rel)).css;
const CSS = [
    compile("../../element/modal.scss"),
    compile("./accounts-gallery.scss"),
    compile("../agent/components/InAppLoginPanel.scss"),
    compile("../agent/styles/_activity-log.scss"),
].join("\n");
const TOKENS = `:root{--space-1:4px;--space-1-5:6px;--space-2:8px;--space-3:12px;--space-4:16px;--space-5:20px;--space-6:24px;
--text-sm:12px;--text-md:14px;--zoomfactor:1;--border-color:#444;--tint-2:#222;--main-bg-color:#111;--main-text-color:#eee;
--secondary-text-color:#aaa;--modal-bg-color:#222;--radius-sm:2px;--radius-md:4px;--radius-lg:6px}
*{box-sizing:border-box}body{margin:0;font:13px sans-serif;background:#111;color:#eee}button{font:inherit;white-space:nowrap}`;

const URL_TEXT = "https://claude.ai/oauth/authorize?code=true&client_id=abc&state=" + "x".repeat(260);
const steps = `
<div class="in-app-login-waiting-title">Sign in to Anthropic</div>
<div class="in-app-login-step"><div class="in-app-login-hint" data-hint>Your browser should have opened.</div>
  <div class="in-app-login-url-row"><code class="in-app-login-url-text">${URL_TEXT}</code></div>
  <div class="in-app-login-url-actions"><button data-btn>Open link</button><button data-btn>Copy link</button></div></div>
<div class="in-app-login-step"><div class="in-app-login-callback-row"><input class="in-app-login-url-input" data-first-field>
  <button data-btn>Paste &amp; submit</button><button data-btn>Submit</button></div></div>
<div class="in-app-login-actions"><button data-btn>Cancel login</button><button data-btn>Use terminal instead</button></div>`;

/** A pane `w` × `h` with the dialog over it, at the pane's zoom `z`. */
const dialog = (name: string, z: number, w: number, h: number) =>
    `<div data-case="${name}" style="position:relative;display:flex;flex-direction:column;width:${w}px;height:${h}px;--pane-zoom:${z}">` +
    `<div class="modal-layer-mount modal-layer-mount--fill"><div class="modal-root" data-scope="pane">` +
    `<div class="modal-panel accounts-chooser accounts-chooser--pane"><div class="accounts-chooser-login-body"><div class="in-app-login-waiting">${steps}</div></div></div></div></div></div>`;

/** The docked card under a short transcript, in a pane `h` tall. */
const docked = (name: string, h: number) =>
    `<div class="agent-view" data-case="${name}" style="display:flex;flex-direction:column;width:380px;height:${h}px;overflow:hidden">` +
    `<div data-transcript style="flex:1 1 auto;min-height:0"></div>` +
    `<div class="agent-auth-panel-card"><div class="in-app-login-waiting">${steps}</div></div></div>`;

const MEASURE = `
const out = {};
const num = (n) => Math.round(n * 10) / 10;
for (const el of document.querySelectorAll("[data-case]")) {
    const panel = el.querySelector(".modal-panel, .agent-auth-panel-card");
    const pr = panel.getBoundingClientRect();
    const r = el.getBoundingClientRect();
    const right = Math.max(...[...panel.querySelectorAll("[data-btn]")].map((b) => b.getBoundingClientRect().right));
    const transcript = el.querySelector("[data-transcript]");
    panel.querySelector("[data-first-field]")?.focus();
    out[el.dataset.case] = {
        overflowY: getComputedStyle(panel).overflowY,
        scrollable: panel.scrollHeight > panel.clientHeight,
        inside: pr.top >= r.top - 1 && pr.bottom <= r.bottom + 1,
        hint: num(panel.querySelector(".in-app-login-waiting-title").getBoundingClientRect().height),
        buttonsInside: right <= pr.right + 0.5,
        // Room between the panel's edge and the first line, in the panel's own (zoomed) px.
        inset: num(panel.querySelector(".in-app-login-waiting-title").getBoundingClientRect().left - pr.left),
        panelHeight: num(pr.height),
        transcriptHeight: transcript ? num(transcript.getBoundingClientRect().height) : null,
        // The top stays reachable: scrolling to the top shows the title, which is not cut off.
        topReachable: (panel.scrollTop = 0, panel.querySelector(".in-app-login-waiting-title").getBoundingClientRect().top >= pr.top - 1),
    };
}
const pre = document.createElement("pre"); pre.id = "result"; pre.textContent = JSON.stringify(out); document.body.appendChild(pre);
`;

interface Measured {
    overflowY: string;
    scrollable: boolean;
    inside: boolean;
    hint: number;
    buttonsInside: boolean;
    inset: number;
    panelHeight: number;
    transcriptHeight: number | null;
    topReachable: boolean;
}

describe.skipIf(!BROWSER)("sign-in dialog over an agent pane", () => {
    it("scrolls inside the pane, follows the pane's zoom, wraps its buttons, and the docked card scrolls too", async () => {
        const html =
            `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${CSS}</style>` +
            `${dialog("z1", 1, 380, 260)}${dialog("z2", 2, 380, 260)}${dialog("narrow", 2, 277, 369)}${docked("docked", 400)}` +
            `<script>window.addEventListener("load",()=>{${MEASURE}})</script>`;
        const r = await measureInBrowser<Record<string, Measured>>(BROWSER!, html, { width: 1300, height: 1400 });

        for (const k of ["z1", "z2"]) {
            expect(r[k].overflowY).toBe("auto");
            expect(r[k].scrollable).toBe(true);
            expect(r[k].inside).toBe(true); // never taller than the pane
            expect(r[k].buttonsInside).toBe(true); // the rows wrapped
            expect(r[k].topReachable).toBe(true);
        }
        // Text and controls sit in from the dialog's edge, not against it; the docked card has its own padding.
        expect(r.z1.inset).toBeGreaterThanOrEqual(14);
        expect(r.docked.inset).toBeGreaterThanOrEqual(12);
        // A narrow pane at zoom 2 (about 128 px of room): a button wider than that wraps its label, it is not cut off.
        expect(r.narrow.buttonsInside).toBe(true);

        // The pane's zoom 2 draws the same line twice as tall as zoom 1 does.
        expect(r.z2.hint / r.z1.hint).toBeGreaterThan(1.8);
        expect(r.z2.hint / r.z1.hint).toBeLessThan(2.2);

        // The docked card is capped, scrolls, and leaves the conversation room.
        expect(r.docked.overflowY).toBe("auto");
        expect(r.docked.scrollable).toBe(true);
        expect(r.docked.panelHeight).toBeLessThanOrEqual(0.7 * 400 + 1);
        expect(r.docked.transcriptHeight!).toBeGreaterThan(0);
        expect(r.docked.buttonsInside).toBe(true);
    }, 60_000);
});
