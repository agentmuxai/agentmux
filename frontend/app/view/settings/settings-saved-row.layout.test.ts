// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The "Saved" note must not change the height of its row, measured in a real browser: the note shows on the
 * label's line for about a second and a half, and a row that grew while it showed would move the description and
 * every row below it twice. Skips with no usable browser.
 */

import { join } from "node:path";
import * as sass from "sass";
import { describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser } from "@/app/view/agent/components/layout-browser";

const BROWSER = findBrowser();
const ui = sass.compile(join(__dirname, "../../element/ui/ui.scss")).css;
const settings = sass.compile(join(__dirname, "settings.scss")).css;
const TOKENS = `:root{--text-xs:11px;--text-sm:12px;--space-2:8px;--success-text-color:#2e7d32;--main-text-color:#111;
--duration-linger:1500ms;--ease-standard:ease;--font-weight-medium:500}body{margin:0;font:13px sans-serif}`;
const MEASURE = `
const out = {};
for (const el of document.querySelectorAll("[data-case]")) out[el.dataset.case] = el.getBoundingClientRect().height;
const pre = document.createElement("pre"); pre.id = "result"; pre.textContent = JSON.stringify(out); document.body.appendChild(pre);
`;
const row = (name: string, flag: boolean, extra = "") =>
    `<div data-case="${name}" class="ui-field-label-row"><label class="ui-field-label">Daily cap</label>` +
    `<span class="setting-saved">${flag ? `<span class="setting-saved-flag" style="${extra}">Saved</span>` : ""}</span></div>`;

describe.skipIf(!BROWSER)("settings Saved note", () => {
    it("does not make its row taller while it shows", async () => {
        const html =
            `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${ui}\n${settings}</style>` +
            `${row("rest", false)}${row("showing", true)}` +
            // The sensitivity arm: the flag with the tall line box it would have without its own line-height.
            `${row("tall", true, "line-height:2.5")}` +
            `<script>window.addEventListener("load",()=>{${MEASURE}})</script>`;
        const r = await measureInBrowser<Record<string, number>>(BROWSER!, html, { width: 500, height: 200 });
        expect(r.showing).toBe(r.rest);
        expect(r.tall).toBeGreaterThan(r.rest);
    }, 30_000);
});
