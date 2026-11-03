// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The `chrome` button's look, measured in a real browser: the pane header's icon buttons
 * must keep the look the older widget IconButton gave them (an icon with no line, padding
 * or tint; opacity for rest, hover and disabled; the accent colour when pressed), and a
 * header rule that sets their size must still win. Skips with no usable browser.
 */

import { join } from "node:path";
import * as sass from "sass";
import { describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser } from "@/app/view/agent/components/layout-browser";

const BROWSER = findBrowser();
const css = sass.compile(join(__dirname, "ui.scss")).css;
const TOKENS = `:root{--accent-color:#58c142;--highlight-bg-color:#333;--ui-height:28px}
body{margin:0;font:13px sans-serif}.hdr{display:flex;align-items:center;height:30px}
.hdr .hdr-rule{width:26px;padding:4px 6px;font-size:18px}`;
const MEASURE = `
const out = {};
for (const el of document.querySelectorAll("[data-case]")) {
  const cs = getComputedStyle(el);
  out[el.dataset.case] = { display: cs.display, opacity: cs.opacity, color: cs.color, border: cs.borderTopWidth, padding: cs.paddingLeft, background: cs.backgroundColor, cursor: cs.cursor, width: cs.width, fontSize: cs.fontSize };
}
const pre = document.createElement("pre"); pre.id = "result"; pre.textContent = JSON.stringify(out); document.body.appendChild(pre);
`;
const btn = (c: string, cls: string, attrs = "") => `<button data-case="${c}" type="button" class="ui-button--chrome ${cls}" ${attrs}><i style="display:inline-block;width:1em;height:1em"></i></button>`;
const body = [
    btn("rest", ""),
    btn("disabled", "", "disabled"),
    btn("off", "ui-button--toggle", 'aria-pressed="false"'),
    btn("on", "ui-button--toggle", 'aria-pressed="true"'),
    btn("header", "hdr-rule"),
].join("");

describe.skipIf(!BROWSER)("chrome button", () => {
    it("is an unlined, unpadded icon that shows state as opacity and the accent colour", async () => {
        const html = `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style><div class="hdr">${body}</div><script>window.addEventListener("load",()=>{${MEASURE}})</script>`;
        const r = await measureInBrowser<Record<string, Record<string, string>>>(BROWSER!, html, { width: 400, height: 100 });
        expect(r.rest).toMatchObject({ display: "flex", opacity: "0.85", border: "0px", padding: "0px", background: "rgba(0, 0, 0, 0)", cursor: "default" });
        expect(r.disabled.opacity).toBe("0.45");
        expect(r.off.opacity).toBe("0.85");
        expect(r.on).toMatchObject({ opacity: "1", color: "rgb(88, 193, 66)" });
        // A header rule that sizes its buttons (as the real ones do, through an ancestor) beats the variant's own rules.
        expect(r.header).toMatchObject({ width: "26px", padding: "6px", fontSize: "18px" });
    });
});
