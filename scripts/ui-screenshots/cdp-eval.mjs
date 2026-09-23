// Evaluate a JS expression (may return a Promise) in a CDP page target and
// print the JSON result. Used to run in-page instrumentation (Event Timing,
// long tasks, rAF gaps, mutation rate) against a live dev build.
//
//   node scripts/ui-screenshots/cdp-eval.mjs <port> <targetIdSubstr> <file.js | 'expr'>
import WebSocket from "ws";
import fs from "node:fs";

const [port = "9223", targetSel = "", src = "1+1"] = process.argv.slice(2);
const expression = fs.existsSync(src) ? fs.readFileSync(src, "utf8") : src;
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page" && (t.id.includes(targetSel) || t.url.includes(targetSel)));
if (!page) { console.error("no page target matching", targetSel); process.exit(1); }
console.error("evaluating in", page.title);

const ws = new WebSocket(page.webSocketDebuggerUrl, { perMessageDeflate: false, maxPayload: 1 << 28 });
await new Promise((r, e) => { ws.once("open", r); ws.once("error", e); });
let id = 0; const pending = new Map();
ws.on("message", (m) => { const j = JSON.parse(m); if (j.id && pending.has(j.id)) { pending.get(j.id)(j); pending.delete(j.id); } });
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });

const res = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true, timeout: 120000 });
ws.close();
if (res.result?.exceptionDetails) { console.error(JSON.stringify(res.result.exceptionDetails, null, 1)); process.exit(1); }
const v = res.result?.result?.value;
console.log(typeof v === "string" ? v : JSON.stringify(v, null, 1));
