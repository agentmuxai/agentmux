// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase 4 zoom-units probe. Injects a faithful scroll-container structure
// (.agent-document overflow + absolute-positioned row in a tall virtualizer)
// under a CSS `zoom` ancestor, then reads the layout properties the agent
// pane relies on at zoom 1 / 0.5 / 2. Ratio vs zoom=1 tells us, per property:
//   ratio ~= zoom  -> ZOOMED (device px) -> needs ÷zoom to reach unzoomed CSS px
//   ratio ~= 1     -> UNZOOMED (CSS px)  -> ÷zoom is REDUNDANT/WRONG here
// Settles AgentDocumentVirtualList.tsx:243 ("pending CDP confirmation").
//
// Usage: node tools/tests/ph4-zoom-probe.mjs [cdpPort=9223]
import { WebSocket } from "ws";

class S {
  constructor(u) { this.u = u; this.ws = null; this.id = 1; this.p = new Map(); }
  async connect() {
    await new Promise((res, rej) => {
      this.ws = new WebSocket(this.u, { perMessageDeflate: false });
      this.ws.on("open", res); this.ws.on("error", rej);
      this.ws.on("message", d => {
        const m = JSON.parse(d.toString());
        if (m.id == null) return;
        const p = this.p.get(m.id); if (!p) return;
        this.p.delete(m.id);
        m.error ? p.reject(new Error(m.error.message)) : p.resolve(m.result);
      });
    });
  }
  send(method, params = {}) {
    const id = this.id++;
    return new Promise((res, rej) => { this.p.set(id, { resolve: res, reject: rej }); this.ws.send(JSON.stringify({ id, method, params })); });
  }
  close() { this.ws && this.ws.close(); }
}

const ev = async (s, e) => {
  const r = await s.send("Runtime.evaluate", { expression: e, returnByValue: true });
  if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
  return r.result?.value;
};

const port = process.argv[2] || 9223;
const res = await fetch(`http://127.0.0.1:${port}/json`);
const targets = (await res.json()).filter(t => t.type === "page");
const main = targets.find(t => !/window-pool|pool=1|devtools/.test(t.url || "")) || targets[0];
if (!main) { console.error("no page target"); process.exit(1); }
console.log("page:", main.title, "|", main.url);
const s = new S(main.webSocketDebuggerUrl); await s.connect(); await s.send("Runtime.enable");

const EXPR = `(() => {
  const view = document.createElement('div');
  view.style.cssText = 'position:fixed;left:-9999px;top:0;';
  const doc = document.createElement('div');
  doc.style.cssText = 'height:400px;width:600px;overflow-y:auto;position:relative;';
  const virt = document.createElement('div');
  virt.style.cssText = 'height:2000px;position:relative;';
  const row = document.createElement('div');
  row.style.cssText = 'position:absolute;top:300px;left:0;width:100%;height:100px;';
  virt.appendChild(row); doc.appendChild(virt); view.appendChild(doc); document.body.appendChild(view);
  const m = (z) => {
    view.style.zoom = String(z);
    void doc.offsetHeight;
    doc.scrollTop = 100; void doc.scrollTop;
    const rr = row.getBoundingClientRect();
    const dr = doc.getBoundingClientRect();
    return { zoom: z,
      doc_clientHeight: doc.clientHeight,
      doc_scrollHeight: doc.scrollHeight,
      doc_scrollTop_set100: doc.scrollTop,
      doc_gbcr_height: Math.round(dr.height * 100) / 100,
      row_offsetTop: row.offsetTop,
      row_offsetHeight: row.offsetHeight,
      row_gbcr_height: Math.round(rr.height * 100) / 100 };
  };
  const out = [m(1), m(0.5), m(2)];
  view.remove();
  return out;
})()`;

const out = await ev(s, EXPR);
console.log("\n=== raw ===");
console.log(JSON.stringify(out, null, 2));
const base = out.find(o => o.zoom === 1);
for (const o of out) {
  if (o.zoom === 1) continue;
  console.log(`\n=== zoom ${o.zoom} vs 1 — ratio ~${o.zoom} => ZOOMED(÷zoom); ratio ~1 => UNZOOMED ===`);
  for (const k of Object.keys(o)) {
    if (k === "zoom") continue;
    const ratio = base[k] ? o[k] / base[k] : NaN;
    console.log(`  ${k}: ${o[k]}  (ratio ${ratio.toFixed(3)})`);
  }
}
s.close();
