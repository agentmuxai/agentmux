// Terminal hit-test smoke check — guards the xterm-6 scrollbar / link-layer
// stacking contract that two regressions slipped through (URL hover and
// scrollbar click). Run against a live dev/release instance after ANY change
// to term.scss, termwrap.ts, or the xterm/addon versions.
//
// Asserts, via document.elementFromPoint (exactly what a real click resolves):
//   1. Over the scrollbar lane (scrollbar forced visible) -> `.slider` wins.
//      If `.xterm-link-layer` wins instead, the scrollbar is unclickable.
//   2. Over the text area -> `.xterm-link-layer` wins, and `.xterm-screen` has
//      pointer-events:auto. If not, URL/file hover is dead.
//
// Usage: node cdp-term-smoke.mjs [port]   (default 9223 = dev, 9222 = release)
// Exit 0 = all pass, 1 = a contract broke, 2 = could not connect / no terminal.

const port = process.argv[2] || "9223";

const PAGE_SCRIPT = `(() => {
  const term = document.querySelector('.xterm');
  if (!term) return JSON.stringify({ ok: false, reason: 'no .xterm terminal on page' });
  const screen = term.querySelector('.xterm-screen');
  const linkLayer = term.querySelector('.xterm-link-layer');
  const vbar = term.querySelector('.scrollbar.vertical');
  const slider = vbar?.querySelector('.slider');
  if (!vbar || !slider) return JSON.stringify({ ok: false, reason: 'no xterm-6 overlay scrollbar (.scrollbar.vertical > .slider)' });

  const hit = (x, y) => { const el = document.elementFromPoint(x, y); return el ? (el.tagName + '.' + (el.className?.toString?.().slice(0,30) || '')) : null; };

  // (1) Scrollbar lane — force the (possibly hidden) overlay scrollbar hit-testable
  // WITHOUT touching z-index; the term.scss z-index must do the work on its own.
  const sv = vbar.style.cssText, ss = slider.style.cssText;
  vbar.style.visibility = 'visible'; slider.style.visibility = 'visible';
  const br = vbar.getBoundingClientRect();
  const yMid = Math.round(br.top + br.height / 2);
  const laneHits = [3, 7, 11].map(off => hit(Math.round(br.right - off), yMid));
  vbar.style.cssText = sv; slider.style.cssText = ss;

  // (2) Text area — mid-left of the screen should hit the link layer (hover works).
  const sr = screen.getBoundingClientRect();
  const textHit = hit(Math.round(sr.left + sr.width * 0.25), Math.round(sr.top + sr.height / 2));
  const screenPE = getComputedStyle(screen).pointerEvents;
  const vbarZ = getComputedStyle(vbar).zIndex;
  const linkZ = linkLayer ? getComputedStyle(linkLayer).zIndex : null;

  const scrollbarClickable = laneHits.every(h => h && h.includes('slider'));
  const hoverWorks = !!textHit && textHit.includes('link-layer') && screenPE === 'auto';
  return JSON.stringify({
    ok: scrollbarClickable && hoverWorks,
    scrollbarClickable, hoverWorks,
    laneHits, textHit, screenPE, vbarZ, linkZ,
  });
})()`;

async function pickTarget() {
  const res = await fetch(`http://localhost:${port}/json/list`);
  const targets = await res.json();
  // Prefer a workspace/tab page on the vite dev origin; fall back to any page.
  return (
    targets.find((t) => t.type === "page" && /workspace|tab/i.test(t.title || "")) ||
    targets.find((t) => t.type === "page" && (t.url || "").includes("localhost")) ||
    targets.find((t) => t.type === "page")
  );
}

async function evalOnPage(wsUrl, expression) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(wsUrl);
    let id = 0;
    const pending = new Map();
    const send = (method, params) =>
      new Promise((res) => { const myId = ++id; pending.set(myId, res); ws.send(JSON.stringify({ id: myId, method, params })); });
    ws.addEventListener("message", (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
    });
    ws.addEventListener("open", async () => {
      await send("Runtime.enable", {});
      const r = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
      ws.close();
      if (r.result?.exceptionDetails) reject(new Error(JSON.stringify(r.result.exceptionDetails)));
      else resolve(r.result?.result?.value);
    });
    ws.addEventListener("error", (e) => reject(new Error(e.message || "ws error")));
  });
}

(async () => {
  let target;
  try {
    target = await pickTarget();
  } catch (e) {
    console.error(`could not reach CDP on :${port} (${e.message}). Is the app running? dev=9223, release=9222.`);
    process.exit(2);
  }
  if (!target?.webSocketDebuggerUrl) {
    console.error("no debuggable page target found");
    process.exit(2);
  }
  let out;
  try {
    out = JSON.parse(await evalOnPage(target.webSocketDebuggerUrl, PAGE_SCRIPT));
  } catch (e) {
    console.error("page eval failed:", e.message);
    process.exit(2);
  }
  if (out.reason) {
    console.error(`skip: ${out.reason}`);
    process.exit(2);
  }
  const mark = (b) => (b ? "PASS" : "FAIL");
  console.log(`[${mark(out.scrollbarClickable)}] scrollbar clickable  (lane hits: ${JSON.stringify(out.laneHits)}; vbar z=${out.vbarZ}, link-layer z=${out.linkZ})`);
  console.log(`[${mark(out.hoverWorks)}] link hover works     (text hit: ${out.textHit}; .xterm-screen pointer-events=${out.screenPE})`);
  if (out.ok) { console.log("PASS — terminal hit-test contract intact"); process.exit(0); }
  console.error("FAIL — terminal hit-test contract broken");
  process.exit(1);
})();
