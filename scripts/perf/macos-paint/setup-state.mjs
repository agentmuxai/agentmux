// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Build the Windows analysis's "loaded" state: N window tabs x M terminals.
import { connectPage } from "../../ui-screenshots/lib/cdp-client.mjs";
const tabsWanted = +(process.argv[2] ?? 5), termsPer = +(process.argv[3] ?? 4), view = process.argv[4] ?? "term";
const { session: page } = await connectPage(9223, "window_transparent");
const ev = async (e) => { const r = await page.send("Runtime.evaluate", { expression: e, awaitPromise: true, returnByValue: true }); if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 400)); return r.result.value; };
const T = await ev(`(performance.getEntriesByType("resource").map((e) => e.name).find((n) => n.includes("/frontend/layout/index.ts")) || "").replace(/^[^?]*/, "")`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
await ev(`(async () => { window.__m = { ta: await import("/frontend/app/store/tab-actions.ts${T}"), g: await import("/frontend/app/store/global.ts${T}") }; return true; })()`);
const count = () => ev(`document.querySelectorAll("[data-blockid]").length`);
console.log("blocks before:", await count());
for (let t = 0; t < tabsWanted; t++) {
    if (t > 0) { await ev(`window.__m.ta.createTab(); true`); await sleep(1500); }
    for (let i = 0; i < termsPer; i++) { await ev(`window.__m.g.createBlock({ meta: { view: "${view}"${view === "term" ? ', controller: "shell"' : ""} } }).then(() => true)`); await sleep(700); }
    console.log(`tab ${t + 1}/${tabsWanted} filled; blocks in DOM:`, await count());
}
await sleep(1500);
console.log("elements:", await ev(`document.getElementsByTagName("*").length`), "tabs:", await ev(`document.querySelectorAll('.tab, [data-tabid]').length`));
process.exit(0);
