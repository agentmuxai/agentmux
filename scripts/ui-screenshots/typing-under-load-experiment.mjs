// Hands-free A/B: same streaming load, LoAF-attributed frames WITHOUT typing
// vs WITH synthetic key-repeat typing. Optionally kicks off the load by sending
// a prompt into every agent composer in the page first.
//
//   node scripts/ui-screenshots/typing-under-load-experiment.mjs <port> <targetIdSubstr> \
//        [--send "<prompt>"] [--ramp 40] [--secs 15] [--kps 20] [--typeInto "Maka"]
import WebSocket from "ws";

const args = process.argv.slice(2);
const port = args[0] ?? "9223", targetSel = args[1] ?? "";
const opt = (k, d) => { const i = args.indexOf(k); return i > 0 ? args[i + 1] : d; };
const prompt = opt("--send", null), ramp = Number(opt("--ramp", "40")), secs = Number(opt("--secs", "15"));
const kps = Number(opt("--kps", "20")), typeInto = opt("--typeInto", "Maka"), ch = "f";

const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page" && (t.id.includes(targetSel) || t.url.includes(targetSel)));
if (!page) { console.error("no page target"); process.exit(1); }
const ws = new WebSocket(page.webSocketDebuggerUrl, { perMessageDeflate: false, maxPayload: 1 << 28 });
await new Promise((r, e) => { ws.once("open", r); ws.once("error", e); });
let id = 0; const pending = new Map();
ws.on("message", (m) => { const j = JSON.parse(m); if (j.id && pending.has(j.id)) { pending.get(j.id)(j); pending.delete(j.id); } });
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const evalIn = async (expression) => {
  const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.result?.exceptionDetails) throw new Error(r.result.exceptionDetails.exception?.description || JSON.stringify(r.result.exceptionDetails));
  return r.result?.result?.value;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (...a) => console.error(new Date().toISOString().slice(11, 19), ...a);

const vis = await evalIn(`document.visibilityState`);
if (vis !== "visible") { console.error(`page is ${vis} — restore the window first`); process.exit(2); }

// --- optional: send the prompt into every composer ---
const focusComposer = (name) => evalIn(`(()=>{const tas=[...document.querySelectorAll(".agent-view textarea, textarea")];const ta=tas.find(t=>t.placeholder.includes(${JSON.stringify(name)}));if(!ta)return false;ta.focus();return document.activeElement===ta})()`);
if (prompt) {
  const names = await evalIn(`[...document.querySelectorAll("textarea")].map(t=>t.placeholder.replace("Send message to ","").replace("...",""))`);
  log("composers:", names.join(", "));
  for (const n of names) {
    if (!(await focusComposer(n))) { log("could not focus", n); continue; }
    await send("Input.insertText", { text: prompt });
    await sleep(150);
    await send("Input.dispatchKeyEvent", { type: "keyDown", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13, text: "\r" });
    await send("Input.dispatchKeyEvent", { type: "keyUp", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 });
    await sleep(300);
    log("sent to", n);
  }
  log(`ramping ${ramp}s for streams to get heavy ...`);
  await sleep(ramp * 1000);
}

// --- LoAF recorder (shared) ---
const ARM = `(()=>{const P=window.__exp={t0:performance.now(),loafs:[],mut:0,keys:0,gaps:[]};
  P.kh=()=>P.keys++; document.addEventListener("keydown",P.kh,true);
  P.mo=new MutationObserver(m=>P.mut+=m.length); P.mo.observe(document,{subtree:true,childList:true,characterData:true,attributes:true});
  P.po=new PerformanceObserver(l=>{for(const e of l.getEntries()){P.loafs.push({start:+(e.startTime-P.t0).toFixed(0),dur:+e.duration.toFixed(0),blocking:+e.blockingDuration.toFixed(0),render:e.renderStart?+(e.startTime+e.duration-e.renderStart).toFixed(0):0,styleLayout:e.styleAndLayoutStart?+(e.startTime+e.duration-e.styleAndLayoutStart).toFixed(0):0,scripts:(e.scripts||[]).map(s=>({inv:(s.invoker||"").slice(0,60),fn:(s.sourceFunctionName||"").slice(0,40),url:(s.sourceURL||"").replace(/^.*\\/(node_modules\\/\\.vite\\/deps|app)\\//,"$1/").slice(0,55),line:s.sourceCharPosition,dur:+s.duration.toFixed(1),fl:+s.forcedStyleAndLayoutDuration.toFixed(1)}))})}});
  P.po.observe({type:"long-animation-frame"});
  let last=performance.now(); P.raf=true; (function f(t){P.gaps.push(t-last); last=t; if(P.raf) requestAnimationFrame(f)})(last); return true})()`;
const READ = `(()=>{const P=window.__exp;P.raf=false;P.po.disconnect();P.mo.disconnect();document.removeEventListener("keydown",P.kh,true);
  const L=P.loafs; const scr=l=>l.scripts.reduce((a,s)=>a+s.dur,0); const sum=f=>+L.reduce((a,l)=>a+f(l),0).toFixed(0);
  const q=(arr,p)=>{if(!arr.length)return null;const s=[...arr].sort((x,y)=>x-y);return +s[Math.min(s.length-1,Math.floor(p*s.length))].toFixed(1)};
  const byInv=new Map(); for(const l of L) for(const s of l.scripts){const k=s.inv+" | "+s.fn+" @ "+s.url+":"+s.line; const v=byInv.get(k)||{n:0,dur:0,fl:0}; v.n++; v.dur+=s.dur; v.fl+=s.fl; byInv.set(k,v)}
  return {elapsedMs:+(performance.now()-P.t0).toFixed(0),keydowns:P.keys,mutations:P.mut,frames:P.gaps.length,
    rafGapMs:{p50:q(P.gaps,.5),p95:q(P.gaps,.95),max:q(P.gaps,1),over50:P.gaps.filter(g=>g>50).length},
    loaf:{count:L.length,totalMs:sum(l=>l.dur),blockingMs:sum(l=>l.blocking),scriptMs:sum(scr),forcedLayoutInScriptsMs:sum(l=>l.scripts.reduce((a,s)=>a+s.fl,0)),renderPhaseMs:sum(l=>l.render),ofWhichStyleLayoutMs:sum(l=>l.styleLayout),maxMs:L.length?Math.max(...L.map(l=>l.dur)):0},
    topScripts:[...byInv].map(([k,v])=>({k,n:v.n,ms:+v.dur.toFixed(0),forcedLayoutMs:+v.fl.toFixed(0)})).sort((a,b)=>b.ms-a.ms).slice(0,16),
    worst3:[...L].sort((a,b)=>b.dur-a.dur).slice(0,3).map(l=>({dur:l.dur,blocking:l.blocking,scriptMs:+scr(l).toFixed(0),renderMs:l.render,styleLayoutMs:l.styleLayout,top:[...l.scripts].sort((a,b)=>b.dur-a.dur).slice(0,3).map(s=>s.inv.slice(0,32)+"|"+s.fn+"@"+s.url.slice(-26)+" "+s.dur+"ms fl="+s.fl)}))}})()`;

const results = {};
// A: no typing
log(`A) ${secs}s streaming, NO typing`);
await evalIn(ARM); await sleep(secs * 1000); results.A_streaming_only = await evalIn(READ);

// B: synthetic typing into the chosen composer
if (!(await focusComposer(typeInto))) { log("could not focus composer", typeInto); }
log(`B) ${secs}s streaming + synthetic typing into ${typeInto} at ${kps}/s`);
await evalIn(ARM);
const n = secs * kps, interval = 1000 / kps, kc = ch.toUpperCase().charCodeAt(0), t0 = Date.now();
for (let i = 0; i < n; i++) {
  const w = t0 + i * interval - Date.now(); if (w > 0) await sleep(w);
  send("Input.dispatchKeyEvent", { type: "keyDown", key: ch, code: `Key${ch.toUpperCase()}`, windowsVirtualKeyCode: kc, nativeVirtualKeyCode: kc, text: ch, unmodifiedText: ch, autoRepeat: i > 0 });
  send("Input.dispatchKeyEvent", { type: "keyUp", key: ch, code: `Key${ch.toUpperCase()}`, windowsVirtualKeyCode: kc, nativeVirtualKeyCode: kc });
}
await sleep(800);
results.B_streaming_plus_typing = await evalIn(READ);
// clean up typed text
await evalIn(`(()=>{const a=document.activeElement;if(a&&a.tagName==="TEXTAREA"){const m=a.value.match(/${ch}+$/);if(m){a.value=a.value.slice(0,-m[0].length);a.dispatchEvent(new Event("input",{bubbles:true}))}}return true})()`);
ws.close();
console.log(JSON.stringify(results, null, 1));
