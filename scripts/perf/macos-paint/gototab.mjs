// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
import { connectPage } from "../../ui-screenshots/lib/cdp-client.mjs";
const idx = +process.argv[2];
const { session: page } = await connectPage(9223, "window_transparent");
const ev = async (e) => { const r = await page.send("Runtime.evaluate", { expression: e, awaitPromise: true, returnByValue: true }); if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 300)); return r.result.value; };
console.log(await ev(`(async () => { const ta = await import("/frontend/app/store/tab-actions.ts"); const g = await import("/frontend/app/store/global.ts"); const ws = g.atoms.workspace(); const ids = [...(ws.pinnedtabids ?? []), ...ws.tabids]; await ta.setActiveTab(ids[${idx}]); await new Promise((r) => setTimeout(r, 1500)); return document.title; })()`));
process.exit(0);
