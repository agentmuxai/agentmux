// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Drives a REAL terminal pane with REAL synthetic keystrokes (CDP
// Input.dispatchKeyEvent) to test whether keyboard input survives a
// full-screen alt-screen program (vim). Ground truth is the FILE vim
// writes: if the typed text lands on disk, every keystroke reached the
// PTY. No canvas/DOM scraping — the WebGL renderer paints to a canvas
// that cannot be read back as text.
//
// Usage: node tools/tests/vim-freeze-probe.mjs [--cdp-port 9223] [--recon]

import { WebSocket } from "ws";
import { readFileSync, existsSync, unlinkSync } from "node:fs";

const args = process.argv.slice(2);
const getArg = (n, d) => { const i = args.indexOf(n); return i >= 0 && args[i + 1] ? args[i + 1] : d; };
const CDP_PORT = parseInt(getArg("--cdp-port", "9223"), 10);
const RECON = args.includes("--recon");
const EVAL = getArg("--eval");
const CONTROL = args.includes("--control");
const BLOCK = getArg("--block");
const CMD = getArg("--cmd");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function findPageTarget(port) {
    const res = await fetch(`http://127.0.0.1:${port}/json`);
    const targets = await res.json();
    const pages = targets.filter((t) => t.type === "page" && t.webSocketDebuggerUrl);
    if (!pages.length) throw new Error(`no page targets on :${port}`);
    console.log(`[cdp] ${pages.length} page target(s):`);
    for (const p of pages) console.log(`      - ${p.title} :: ${p.url.slice(0, 90)}`);
    // Skip pre-warmed pool windows (`pool=1` / `pane-pool=1`) — they are
    // invisible spares, not the window a human is looking at.
    const main = pages.find((p) => !/[?&](pane-)?pool=1/.test(p.url));
    if (!main) throw new Error("only pool windows found; no main window");
    return main;
}

class Session {
    constructor(wsUrl) { this.wsUrl = wsUrl; this.nextId = 1; this.pending = new Map(); }
    connect() {
        return new Promise((resolve, reject) => {
            this.ws = new WebSocket(this.wsUrl, { perMessageDeflate: false, maxPayload: 64 * 1024 * 1024 });
            this.ws.on("open", resolve);
            this.ws.on("error", reject);
            this.ws.on("message", (d) => {
                const m = JSON.parse(d.toString());
                const p = this.pending.get(m.id);
                if (p) { this.pending.delete(m.id); m.error ? p.reject(new Error(JSON.stringify(m.error))) : p.resolve(m.result); }
            });
        });
    }
    send(method, params = {}) {
        const id = this.nextId++;
        return new Promise((resolve, reject) => {
            this.pending.set(id, { resolve, reject });
            this.ws.send(JSON.stringify({ id, method, params }));
        });
    }
    close() { this.ws?.close(); }
}

async function evaluate(s, expression) {
    const r = await s.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
    if (r.exceptionDetails) throw new Error(`eval failed: ${r.exceptionDetails.text}`);
    return r.result?.value;
}

// A real keystroke: keydown -> char -> keyup, same shape a human produces.
async function key(s, { key, code, text, keyCode, modifiers = 0 }) {
    const base = { key, code, windowsVirtualKeyCode: keyCode, nativeVirtualKeyCode: keyCode, modifiers };
    await s.send("Input.dispatchKeyEvent", { type: "keyDown", ...base, text });
    if (text) await s.send("Input.dispatchKeyEvent", { type: "char", ...base, text, unmodifiedText: text });
    await s.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
}

// xterm.js takes printable characters from the helper textarea's `input`
// event, NOT from keydown — keydown only carries the special keys it encodes
// itself (Enter, Escape, arrows, Ctrl-chords). Dispatching keydown+char for
// letters therefore delivers real DOM events that xterm correctly ignores,
// and the PTY sees only the Enter. Verified against the live PTY scrollback:
// the shell logged a bare prompt + `exit=failure;status=1`, i.e. Enter with
// an empty command line. So printable text goes through Input.insertText
// (which fires the textarea `input` event a real keypress would) and special
// keys keep the genuine keydown path.
async function typeString(s, str, perKeyMs = 28) {
    let buf = "";
    const flush = async () => {
        if (!buf) return;
        await s.send("Input.insertText", { text: buf });
        buf = "";
        await sleep(perKeyMs);
    };
    for (const ch of str) {
        if (ch === "\n") { await flush(); await key(s, { key: "Enter", code: "Enter", text: "\r", keyCode: 13 }); await sleep(perKeyMs); }
        else buf += ch;
    }
    await flush();
}

async function recon(s) {
    const info = await evaluate(s, `(() => {
        const q = (sel) => Array.from(document.querySelectorAll(sel));
        return JSON.stringify({
            url: location.href,
            xtermCount: q('.xterm').length,
            xtermTextarea: q('.xterm-helper-textarea').length,
            blocks: q('[data-blockid]').slice(0, 12).map(e => ({
                id: e.getAttribute('data-blockid'),
                view: e.getAttribute('data-view') || e.className.slice(0, 60),
                hasXterm: !!e.querySelector('.xterm'),
                rect: (r => ({w: Math.round(r.width), h: Math.round(r.height)}))(e.getBoundingClientRect()),
            })),
            activeEl: document.activeElement?.className?.slice(0,80) || document.activeElement?.tagName,
        }, null, 2);
    })()`);
    console.log("[recon]\n" + info);
    return JSON.parse(info);
}

const main = async () => {
    const target = await findPageTarget(CDP_PORT);
    const s = new Session(target.webSocketDebuggerUrl);
    await s.connect();
    await s.send("Runtime.enable");
    console.log(`[cdp] connected: ${target.title}`);

    if (EVAL) {
        const out = await evaluate(s, EVAL);
        console.log(typeof out === "string" ? out : JSON.stringify(out, null, 2));
        s.close(); return;
    }
    let state = await recon(s);
    if (RECON) { s.close(); return; }

    // The renderer re-renders (Vite HMR, layout settle), so a single sample
    // can catch an empty frame. Poll until a terminal actually exists.
    let term = BLOCK ? state.blocks.find((b) => b.id === BLOCK && b.hasXterm) : state.blocks.find((b) => b.hasXterm);
    for (let i = 0; !term && i < 20; i++) {
        await sleep(1500);
        state = await evaluate(s, `(() => JSON.stringify({blocks: Array.from(document.querySelectorAll('[data-blockid]')).map(e => ({id: e.getAttribute('data-blockid'), hasXterm: !!e.querySelector('.xterm'), rect: (r => ({w: Math.round(r.width), h: Math.round(r.height)}))(e.getBoundingClientRect())}))}))()`).then(JSON.parse);
        term = BLOCK ? state.blocks.find((b) => b.id === BLOCK && b.hasXterm && b.rect.w > 100)
                     : state.blocks.find((b) => b.hasXterm && b.rect.w > 100);
        process.stdout.write(`\r[wait] for terminal pane… ${i + 1}/20 (blocks=${state.blocks.length})`);
    }
    console.log("");
    if (!term) {
        console.log("\n!! no terminal pane found — open one first (recon shows what exists)");
        s.close(); process.exit(2);
    }
    console.log(`\n[probe] using terminal block ${term.id} (${term.rect.w}x${term.rect.h})`);

    // Focus the pane the way a human does: a REAL mouse click on the
    // terminal surface. `el.focus()` sets DOM focus but does not drive the
    // app's own focused-block routing, so keystrokes arrive at the textarea
    // and are then discarded — which is exactly how the first version of
    // this harness produced a false "freeze reproduced".
    const box = await evaluate(s, `(() => {
        const el = document.querySelector('[data-blockid="${term.id}"] .xterm-screen')
               || document.querySelector('[data-blockid="${term.id}"] .xterm');
        if (!el) return null;
        const r = el.getBoundingClientRect();
        return JSON.stringify({ x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) });
    })()`).then((v) => (v ? JSON.parse(v) : null));
    if (!box) { console.log("!! no xterm surface to click"); s.close(); process.exit(2); }
    const isFocused = () => evaluate(s, `(() => {
        const el = document.querySelector('[data-blockid="${term.id}"] .xterm-helper-textarea');
        return document.activeElement === el;
    })()`);
    let focused = false;
    for (let i = 0; i < 6 && !focused; i++) {
        for (const type of ["mousePressed", "mouseReleased"]) {
            await s.send("Input.dispatchMouseEvent", { type, x: box.x, y: box.y, button: "left", clickCount: 1 });
            await sleep(60);
        }
        await sleep(400);
        focused = await isFocused();
        if (!focused) {
            await evaluate(s, `(() => { const el = document.querySelector('[data-blockid="${term.id}"] .xterm-helper-textarea'); el && el.focus(); return 1; })()`);
            await sleep(300);
            focused = await isFocused();
        }
    }
    console.log(`[probe] click@(${box.x},${box.y}) focused=${focused}`);
    if (!focused) { console.log("!! could not focus the terminal — aborting rather than reporting a false freeze"); s.close(); process.exit(2); }

    if (CMD) {
        console.log(`[cmd] typing into ${term.id}: ${CMD}`);
        await typeString(s, CMD + "\n");
        console.log("[cmd] sent");
        s.close(); return;
    }

    if (CONTROL) {
        // Control: no vim, no alt-screen. Just a plain shell command that
        // writes a file. If THIS fails, the harness is broken — not vim.
        const st = Date.now();
        const f = `/tmp/opaz-control-${st}.txt`;
        console.log(`[control] typing: echo CTRL_${st} > ${f}`);
        await typeString(s, `echo CTRL_${st} > ${f}\n`);
        await sleep(2500);
        const got = existsSync(f) && readFileSync(f, "utf8").includes(`CTRL_${st}`);
        console.log(`\n=== CONTROL RESULT ===`);
        console.log(`file exists : ${existsSync(f)}`);
        console.log(got ? "CONTROL PASS: plain keystrokes reach the shell — harness works"
                        : "CONTROL FAIL: even plain keystrokes do not land — HARNESS IS BROKEN");
        s.close(); process.exit(got ? 0 : 1);
    }

    const stamp = Date.now();
    const outFile = `/tmp/opaz-vim-probe-${stamp}.txt`;
    const sentinel = `PROBE_OK_${stamp}`;
    if (existsSync(outFile)) unlinkSync(outFile);

    console.log(`[probe] typing: vim ${outFile}`);
    await typeString(s, `vim ${outFile}\n`);
    await sleep(2500); // let vim enter the alternate screen and paint

    console.log(`[probe] typing insert-mode text (the real test)`);
    await typeString(s, "i");
    await sleep(400);
    await typeString(s, sentinel);
    await sleep(400);
    await key(s, { key: "Escape", code: "Escape", keyCode: 27 });
    await sleep(400);
    await typeString(s, ":wq\n");
    await sleep(2000);

    const ok = existsSync(outFile) && readFileSync(outFile, "utf8").includes(sentinel);
    console.log(`\n=== RESULT ===`);
    console.log(`file exists : ${existsSync(outFile)}`);
    if (existsSync(outFile)) console.log(`contents    : ${JSON.stringify(readFileSync(outFile, "utf8").slice(0, 120))}`);
    console.log(ok ? "VERDICT: keystrokes reached vim and were written — NO FREEZE"
                   : "VERDICT: keystrokes did NOT survive — FREEZE REPRODUCED");
    s.close();
    process.exit(ok ? 0 : 1);
};

main().catch((e) => { console.error("probe error:", e.message); process.exit(3); });
