// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent pane's question panel in a narrow, zoomed pane, measured in a real browser (#47821). In a 372 px wide,
 * 744 px tall pane at 150% zoom the header ran off the right edge, "Accept Recommended" was clipped at the bottom
 * and the options had no height: the panel had a third of the pane, and its own header and wrapped buttons took all
 * of it.
 *
 * This compiles the real `AgentQuestionPanel.scss`, `pane-requests.scss` and the Button styles, lays the panel out
 * the way `QuestionPanelBody` does inside the pane's request stack, and measures what the person can see. It skips,
 * rather than fails, on a machine with no usable browser (layout-browser.ts findBrowser).
 */

import { join } from "node:path";
import * as sass from "sass";
import { beforeAll, describe, expect, it } from "vitest";
import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "./layout-browser";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const css = [
    join(__dirname, "AgentQuestionPanel.scss"),
    join(__dirname, "../pane-requests/pane-requests.scss"),
    join(__dirname, "../../../element/ui/ui.scss"),
]
    .map((f) => sass.compile(f).css)
    .join("\n");

const TOKENS = `:root{
  --main-text-color:#ddd;--secondary-text-color:#999;--accent-color:#58c142;--accent-color-rgb:88,193,66;
  --attention-color:#ff2bd6;--border-color:#444;--modal-bg-color:#1c1c1c;--main-bg-color:#111;--hover-bg-color:#333;
  --form-element-bg-color:#111;--warning-text-color:#e0b000;--error-text-color:#eb755d;--highlight-bg-color:#333;
  --font-sans:Arial,sans-serif;--text-2xs:10px;--text-xs:11px;--text-sm:12px;--text-base:13px;--text-md:14px;
  --font-weight-medium:500;--leading-tight:1.2;
  --z-over-content:1;--space-1:4px;--space-1-5:6px;--space-2:8px;--space-3:12px;--space-4:16px;--radius-sm:0;--radius-md:0;
  --shadow-lg:0 12px 24px rgba(0,0,0,.35);--duration-quick:120ms;--duration-decorative-instant:80ms;--ease-standard:ease;
  --ui-font-size:12px;--ui-font-weight:500;--ui-pad-y:4px;--ui-pad-x:12px;--ui-gap:6px;--ui-height:26px;--ui-icon-size:12px;
}
body{margin:0;background:#000;color:#ddd;font:13px Arial,sans-serif}
/* The agent pane's own font: monospace at --font-agent-size, which wraps far more than Arial (Clare's look check). */
.view,.view button,.view input{font-family:Consolas,'DejaVu Sans Mono','Liberation Mono',monospace}
.case{margin:0 0 8px;overflow:hidden}`;

/** What the agent pane gives the stack: a column with the transcript above and the composer below. The pane's zoom is
 *  CSS `zoom`, as on `.agent-view-zoomed`: the box is laid out at 1/zoom of the size it shows at. */
function pane(id: string, width: number, height: number, zoom: number): string {
    const option = (label: string, desc: string, recommended = false) =>
        `<label class="agent-question-panel-option${recommended ? " agent-question-panel-option--recommended" : ""}"><input type="radio" name="${id}"><span class="agent-question-panel-option-body"><span class="agent-question-panel-option-label">${label}</span><span class="agent-question-panel-option-desc">${desc}</span></span></label>`;
    return `<div class="case" style="width:${width}px;height:${height}px"><div class="view" id="${id}" style="zoom:${zoom};width:${width / zoom}px;height:${height / zoom}px;display:flex;flex-direction:column;overflow:hidden;container-type:inline-size;container-name:agent-pane;font-size:15px">
<div style="flex:1 1 auto;min-height:0;background:#181818"></div>
<div class="agent-request-stack">
<div class="agent-question-panel" role="group">
<div class="agent-question-panel-header"><span class="agent-question-panel-icon">❓</span><span class="agent-question-panel-title">The agent is asking</span><span class="agent-question-panel-countdown">Auto-selects recommended in 28s</span></div>
<div class="agent-question-panel-scroll"><fieldset class="agent-question-panel-q"><legend class="agent-question-panel-q-prompt"><span style="border:1px solid #58c142;border-radius:9px;padding:0 6px">Channel</span>Which release channel should the made-up docs preview be published to?</legend>
<div class="agent-question-panel-options">${option("Stacked (Recommended)", "One column, answers under the options", true)}${option("Side by side", "Options left, preview right")}${option("Tabs", "One question per tab")}
<label class="agent-question-panel-other"><span class="agent-question-panel-other-label">Other</span><input class="agent-question-panel-other-input ui-input" placeholder="Type a custom answer…"></label></div></fieldset></div>
<div class="agent-question-panel-actions"><button class="ui-button agent-question-panel-btn">Cancel</button><button class="ui-button agent-question-panel-btn">Accept Recommended</button><button class="ui-button ui-tone-attention agent-question-panel-btn" disabled>Submit answer</button></div>
</div></div>
<div style="flex:none;height:90px;background:#222"></div>
</div></div>`;
}

/** Pane sizes as the person sees them (before the zoom), then the zoom. */
const CASES: Array<{ id: string; width: number; height: number; zoom: number }> = [
    { id: "narrow-100", width: 372, height: 744, zoom: 1 },
    { id: "narrow-150", width: 372, height: 744, zoom: 1.5 },
    { id: "narrow-200", width: 372, height: 744, zoom: 2 },
    { id: "wide-100", width: 700, height: 744, zoom: 1 },
];

const MEASURE = `
// What of an element the person can see: its box cut by every ancestor that clips or scrolls.
const visible = (el) => {
  let r = el.getBoundingClientRect();
  let box = { left: r.left, right: r.right, top: r.top, bottom: r.bottom };
  for (let p = el.parentElement; p; p = p.parentElement) {
    const cs = getComputedStyle(p);
    if (cs.overflowX === 'visible' && cs.overflowY === 'visible') continue;
    const b = p.getBoundingClientRect();
    box = { left: Math.max(box.left, b.left), right: Math.min(box.right, b.right), top: Math.max(box.top, b.top), bottom: Math.min(box.bottom, b.bottom) };
  }
  return { width: Math.max(0, box.right - box.left), height: Math.max(0, box.bottom - box.top), full: { width: r.width, height: r.height } };
};
const result = {};
for (const view of document.querySelectorAll('.view')) {
  const q = (s) => view.querySelector(s);
  const header = q('.agent-question-panel-header');
  const hb = header.getBoundingClientRect();
  const actions = q('.agent-question-panel-actions');
  const buttons = [...actions.querySelectorAll('button')].map((b) => visible(b));
  const stack = q('.agent-request-stack');
  const viewBox = view.getBoundingClientRect();
  result[view.id] = {
    view: { width: viewBox.width, height: viewBox.height },
    stack: { height: stack.getBoundingClientRect().height, scrollWidth: stack.scrollWidth, clientWidth: stack.clientWidth },
    panel: { scrollWidth: q('.agent-question-panel').scrollWidth, clientWidth: q('.agent-question-panel').clientWidth },
    headerOverflow: header.scrollWidth - header.clientWidth,
    headerChildrenPastEdge: [...header.children].filter((c) => c.getBoundingClientRect().right > hb.right + 0.5).length,
    headerVisible: visible(header),
    options: visible(q('.agent-question-panel-options')),
    buttons,
  };
}
const out = document.createElement('pre'); out.id = 'result'; out.textContent = JSON.stringify(result); document.body.appendChild(out);`;

interface Seen {
    width: number;
    height: number;
    full: { width: number; height: number };
}
interface Measured {
    view: { width: number; height: number };
    stack: { height: number; scrollWidth: number; clientWidth: number };
    panel: { scrollWidth: number; clientWidth: number };
    headerOverflow: number;
    headerChildrenPastEdge: number;
    headerVisible: Seen;
    options: Seen;
    buttons: Seen[];
}

let measured: Record<string, Measured> = {};

beforeAll(async () => {
    if (!BROWSER) return;
    const html = CASES.map((c) => pane(c.id, c.width, c.height, c.zoom)).join("");
    measured = await measureInBrowser<Record<string, Measured>>(
        BROWSER,
        `<!doctype html><meta charset="utf-8"><style>${TOKENS}\n${css}</style>${html}<script>window.addEventListener("load",()=>{${MEASURE}})</script>`,
        { width: 800, height: 3300 }
    );
}, 90_000);

describe.skipIf(!BROWSER)("Question panel in a narrow, zoomed pane", () => {
    for (const c of CASES) {
        describe(c.id, () => {
            it("keeps the header inside the panel", () => {
                const m = measured[c.id];
                expect(m.headerOverflow, JSON.stringify(m)).toBeLessThanOrEqual(1);
                expect(m.headerChildrenPastEdge, JSON.stringify(m)).toBe(0);
            });

            it("shows every answer button whole, without scrolling", () => {
                const m = measured[c.id];
                for (const b of m.buttons) {
                    expect(b.height, JSON.stringify(m)).toBeGreaterThanOrEqual(b.full.height - 1);
                    expect(b.width, JSON.stringify(m)).toBeGreaterThanOrEqual(b.full.width - 1);
                }
            });

            it("leaves the options room to be read", () => {
                const m = measured[c.id];
                expect(m.options.height, JSON.stringify(m)).toBeGreaterThanOrEqual(80);
            });

            it("never grows past the pane", () => {
                const m = measured[c.id];
                expect(m.stack.scrollWidth, JSON.stringify(m)).toBeLessThanOrEqual(m.stack.clientWidth + 1);
                expect(m.stack.height, JSON.stringify(m)).toBeLessThanOrEqual(m.view.height);
            });

            it("never scrolls the panel sideways", () => {
                const m = measured[c.id];
                expect(m.panel.scrollWidth, JSON.stringify(m)).toBeLessThanOrEqual(m.panel.clientWidth + 1);
            });
        });
    }
});
