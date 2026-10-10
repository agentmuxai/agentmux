// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// setcfg.mjs <key> <true|false|json>: write one settings key through the app's config RPC.
import { connectPage } from "../../ui-screenshots/lib/cdp-client.mjs";
const key = process.argv[2], val = JSON.parse(process.argv[3]);
const { session: page } = await connectPage(9223, "window_transparent");
const ev = async (e) => { const r = await page.send("Runtime.evaluate", { expression: e, awaitPromise: true, returnByValue: true }); if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 300)); return r.result.value; };
await ev(`(async () => { const a = await import("/frontend/app/store/rpc-api/index.ts"); const b = await import("/frontend/app/store/rpc-util.ts"); await a.RpcApi.SetConfigCommand(b.TabRpcClient, { ${JSON.stringify(key)}: ${JSON.stringify(val)} }); return true; })()`);
await new Promise((r) => setTimeout(r, 1000));
console.log(key, "->", JSON.stringify(val), "| effective:", await ev(`(async () => { const g = await import("/frontend/app/store/global.ts"); return g.getSettingsKeyAtom(${JSON.stringify(key)})(); })()`));
process.exit(0);
