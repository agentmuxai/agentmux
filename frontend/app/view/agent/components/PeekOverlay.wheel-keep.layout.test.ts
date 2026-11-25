// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The wheel over a hover panel that scrolls, in a real browser with real wheel input (#48713). A long command in the
 * hover panel scrolls inside it; the wheel over it must not also move the transcript behind it, because the row then
 * leaves the pointer and the panel closes. This bundles the real hand-off (`attachScrollHandoff`) with the options the
 * panel passes, puts it on a fixture shaped like the panel (a header and a capped log inside the panel, rendered
 * outside the transcript), and checks the transcript does not move at the middle or at either end of the log, nor
 * from the momentum of a gesture past the end. It skips, rather than fails, on a machine with no usable browser.
 */

import { join } from "node:path";
import { build } from "vite";
import { beforeAll, describe, expect, it } from "vitest";
import { findBrowser, NO_BROWSER_HINT } from "./layout-browser";
import { withInputPage, type InputPage } from "./layout-browser-input";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const VIEWPORT = { width: 800, height: 600 };
const WHEEL = 100;
/** Over the log: the panel is at (100, 100), 400 wide; its header is 24 high and the log below it 150 high. */
const OVER_LOG = { x: 300, y: 200 };
const OVER_HEADER = { x: 300, y: 112 };

/** The real hand-off, with the app's settings store and Solid's cleanup stood in for: neither matters to a wheel. */
async function bundleHandoff(): Promise<string> {
    const stubs: Record<string, string> = {
        "stub:global": "export const getSettingsKeyAtom = () => () => undefined;",
        "stub:solid": "export const onCleanup = () => {};",
    };
    const out = await build({
        configFile: false,
        logLevel: "silent",
        plugins: [
            {
                name: "stand-ins",
                resolveId: (id) => (id === "@/app/store/global" ? "stub:global" : id === "solid-js" ? "stub:solid" : null),
                load: (id) => stubs[id] ?? null,
            },
        ],
        build: {
            write: false,
            minify: false,
            lib: { entry: join(__dirname, "scroll-handoff.ts"), name: "Handoff", formats: ["iife"], fileName: "handoff" },
        },
    });
    const results = Array.isArray(out) ? out : [out];
    const first = (results[0] as { output: { type: string; code?: string }[] }).output.find((o) => o.type === "chunk");
    return first!.code!;
}

/** `log`: the panel fits and its log scrolls inside it. `panel`: the log has no cap and the panel itself scrolls. */
function fixture(script: string, keepWheel: boolean, shape: "log" | "panel" = "log"): string {
    const logStyle = shape === "log" ? "height:150px;overflow:auto;overscroll-behavior:contain" : "";
    const rows = Array.from({ length: 60 }, (_, i) => `<div style="height:30px">row ${i}</div>`).join("");
    const lines = Array.from({ length: 40 }, (_, i) => `<div style="height:20px;white-space:pre">line ${i}</div>`).join("");
    return `<!doctype html><meta charset="utf-8"><style>body{margin:0}</style>
<div id="scroller" class="agent-document" style="position:fixed;left:0;top:0;width:600px;height:400px;overflow-y:auto">${rows}</div>
<div id="panel" style="position:fixed;left:100px;top:100px;width:400px;height:174px;overflow-y:auto;overscroll-behavior:contain;background:#222;color:#ddd">
  <div id="header" style="height:24px">header</div>
  <div id="log" style="${logStyle}">${lines}</div>
</div>
<script>${script}</script>
<script>
  const scroller = document.getElementById('scroller');
  Handoff.attachScrollHandoff(document.getElementById('panel'), { pane: () => scroller, keepWheel: ${keepWheel} });
  // The log is a preview box of its own, as in the app: it knows no transcript outside the panel.
  Handoff.attachScrollHandoff(document.getElementById('log'));
</script>`;
}

const top = (id: string) => async (page: InputPage) => page.evaluate<number>(`document.getElementById('${id}').scrollTop`);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe.skipIf(!BROWSER)("the wheel over a hover panel that scrolls", () => {
    let script = "";
    beforeAll(async () => {
        script = await bundleHandoff();
    });

    it("scrolls the log and never the transcript: in the middle, at the end, and on momentum past it", async () => {
        const out = await withInputPage(BROWSER!, fixture(script, true), VIEWPORT, async (page) => {
            await page.move(OVER_LOG.x, OVER_LOG.y);
            await page.wheelUntil(OVER_LOG.x, OVER_LOG.y, WHEEL, async () => (await top("log")(page)) > 0);
            const mid = { log: await top("log")(page), scroller: await top("scroller")(page) };
            // Down to the end of the log, and then well past it: ten more notches, as a long gesture would send.
            for (let i = 0; i < 12; i++) {
                await page.wheel(OVER_LOG.x, OVER_LOG.y, WHEEL);
                await sleep(40);
            }
            await sleep(300);
            const end = { log: await top("log")(page), scroller: await top("scroller")(page), panel: await top("panel")(page) };
            // And back up past the top.
            for (let i = 0; i < 16; i++) {
                await page.wheel(OVER_LOG.x, OVER_LOG.y, -WHEEL);
                await sleep(40);
            }
            await sleep(300);
            const start = { log: await top("log")(page), scroller: await top("scroller")(page) };
            return { mid, end, start };
        });
        expect(out.mid.log).toBeGreaterThan(0);
        expect(out.mid.scroller).toBe(0);
        expect(out.end.log).toBeGreaterThan(out.mid.log);
        expect(out.end.scroller).toBe(0);
        expect(out.start.log).toBe(0);
        expect(out.start.scroller).toBe(0);
    }, 120_000);

    it("a panel that scrolls itself holds the wheel at its end too, and the transcript stays", async () => {
        const out = await withInputPage(BROWSER!, fixture(script, true, "panel"), VIEWPORT, async (page) => {
            await page.move(OVER_LOG.x, OVER_LOG.y);
            await page.wheelUntil(OVER_LOG.x, OVER_LOG.y, WHEEL, async () => (await top("panel")(page)) > 0);
            // More notches than the panel has room for: the skid would have run out and handed the rest on.
            for (let i = 0; i < 16; i++) {
                await page.wheel(OVER_LOG.x, OVER_LOG.y, WHEEL);
                await sleep(40);
            }
            await sleep(300);
            return { panel: await top("panel")(page), scroller: await top("scroller")(page) };
        });
        expect(out.panel).toBeGreaterThan(0);
        expect(out.scroller).toBe(0);
    }, 120_000);

    it("a wheel over the header scrolls the log, and the transcript stays", async () => {
        const out = await withInputPage(BROWSER!, fixture(script, true), VIEWPORT, async (page) => {
            await page.move(OVER_HEADER.x, OVER_HEADER.y);
            for (let i = 0; i < 3; i++) {
                await page.wheel(OVER_HEADER.x, OVER_HEADER.y, WHEEL);
                await sleep(60);
            }
            await sleep(300);
            return { log: await top("log")(page), scroller: await top("scroller")(page) };
        });
        expect(out.log).toBeGreaterThan(0);
        expect(out.scroller).toBe(0);
    }, 120_000);
});
