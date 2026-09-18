import { WebSocket } from "ws";
const port = process.argv[2];
const secs = parseInt(process.argv[3] || "90", 10);
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const pages = targets.filter(x => x.type === "page" && x.webSocketDebuggerUrl);
const t = pages.find(p => !/[?&](pane-)?pool=1/.test(p.url)) || pages[0];
console.log("attached:", t.title, "|", t.url.slice(0, 60));
const ws = new WebSocket(t.webSocketDebuggerUrl, { perMessageDeflate: false, maxPayload: 64*1024*1024 });
let id = 1;
const send = (m, p={}) => ws.send(JSON.stringify({ id: id++, method: m, params: p }));
ws.on("open", () => { send("Debugger.enable"); send("Runtime.enable"); send("Log.enable"); console.log("listening…"); });
ws.on("message", d => {
  const m = JSON.parse(d.toString());
  if (m.method === "Runtime.exceptionThrown") {
    const e = m.params.exceptionDetails;
    console.log("\n=== EXCEPTION ===");
    console.log("text :", e.text, "|", (e.exception?.description || "").split("\n")[0]);
    console.log(`site : line ${e.lineNumber} col ${e.columnNumber}`);
    const fr = e.stackTrace?.callFrames || [];
    if (!fr.length) console.log("  (no stack frames)");
    for (const f of fr.slice(0, 15))
      console.log(`  frame ${f.functionName || "(anon)"} ${f.lineNumber}:${f.columnNumber}`);
  }
});
setTimeout(() => { ws.close(); process.exit(0); }, secs * 1000);
