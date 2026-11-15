#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Loads the headless web UI in a real browser under the policy srv serves it with, and fails on
// any policy violation. A clean load must report none, so that one violation means something.
//
//   node scripts/headless-ui-check.mjs --srv <agentmux-srv> --web <built web dir> [--chrome <chrome>] [--port 47450]
//
// It starts a headless srv on an isolated home (no AGENTMUX_* variable passed on), signs a browser in
// the way `agentmux-srv open` does but answering `yes` itself (this is a test of the page, not of the
// person's confirmation), loads the UI, opens a few panes from the command palette, and records every
// `securitypolicyviolation` event, console error and failed request. It also checks what the policy
// is meant to allow and refuse: WebAssembly compiles, `eval` does not. Not part of CI: it needs a
// Chrome and a built web UI (`node scripts/build-web.mjs`). Chrome's own flags: headless, own profile.

import { spawn } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

function arg(name, fallback) {
    const i = process.argv.indexOf(`--${name}`);
    return i > 0 ? process.argv[i + 1] : fallback;
}
const srvExe = arg("srv");
const webDir = arg("web");
const port = Number(arg("port", "47450"));
const chromeExe =
    arg("chrome") ||
    process.env.CHROME ||
    ["C:/Program Files/Google/Chrome/Application/chrome.exe", "C:/Program Files (x86)/Google/Chrome/Application/chrome.exe", "/usr/bin/google-chrome", "/usr/bin/chromium"].find((p) => fs.existsSync(p));
if (!srvExe || !webDir || !chromeExe) {
    console.error("usage: node scripts/headless-ui-check.mjs --srv <agentmux-srv> --web <web dir> [--chrome <chrome>] [--port N]");
    process.exit(2);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "headless-ui-check-"));
const children = [];

// ---- the proof `open` gives and takes (crates/srv/src/server/local_proof.rs) ----
const u64 = (n) => {
    const b = Buffer.alloc(8);
    b.writeBigUInt64BE(BigInt(n));
    return b;
};
function mac(key, label, parts) {
    const h = crypto.createHmac("sha256", key);
    for (const part of [Buffer.from(label), ...parts.map((p) => (Buffer.isBuffer(p) ? p : Buffer.from(p)))]) {
        h.update(u64(part.length));
        h.update(part);
    }
    return h.digest();
}

async function signIn(base, key) {
    const bound = `127.0.0.1:${port}`;
    const nonce = crypto.randomBytes(32).toString("hex");
    const hello = await (await fetch(`${base}/auth/hello`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ nonce }) })).json();
    const expected = mac(key, "amx-srv-hello-v1", [nonce, hello.nonce, bound, hello.declared]).toString("hex");
    if (expected !== hello.proof) throw new Error("srv did not prove it holds the key");
    const ck = mac(key, "amx-channel-v1", [nonce, hello.nonce, bound, hello.declared]);
    let seq = 0;
    const ask = async (route, body) => {
        const bytes = Buffer.from(JSON.stringify(body));
        seq += 1;
        const proof = mac(ck, "amx-req-v1", [u64(seq), "POST", route, crypto.createHash("sha256").update(bytes).digest()]).toString("hex");
        const resp = await fetch(`${base}${route}`, {
            method: "POST",
            headers: { "content-type": "application/json", "x-am-channel": hello.channel, "x-am-seq": String(seq), "x-am-proof": proof },
            body: bytes,
        });
        if (!resp.ok) throw new Error(`${route}: ${resp.status}`);
        const answer = Buffer.from(await resp.arrayBuffer());
        const want = mac(ck, "amx-resp-v1", [u64(seq), (() => { const b = Buffer.alloc(2); b.writeUInt16BE(resp.status); return b; })(), crypto.createHash("sha256").update(answer).digest()]).toString("hex");
        if (resp.headers.get("x-am-proof") !== want) throw new Error(`${route}: the answer did not carry the proof`);
        return JSON.parse(answer.toString());
    };
    return ask;
}

// ---- Chrome over the DevTools protocol ----
async function attach(cdpPort) {
    let target;
    for (let i = 0; i < 50 && !target; i++) {
        try {
            target = (await (await fetch(`http://127.0.0.1:${cdpPort}/json`)).json()).find((t) => t.type === "page");
        } catch {
            /* not up yet */
        }
        if (!target) await sleep(200);
    }
    if (!target) throw new Error("Chrome did not come up");
    const ws = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((r) => (ws.onopen = r));
    let id = 0;
    const waiting = new Map();
    const events = [];
    ws.onmessage = (m) => {
        const d = JSON.parse(m.data);
        if (d.id && waiting.has(d.id)) {
            waiting.get(d.id)(d);
            waiting.delete(d.id);
        } else if (d.method) events.push(d);
    };
    const call = (method, params = {}) => new Promise((res) => { waiting.set(++id, res); ws.send(JSON.stringify({ id, method, params })); });
    return { call, events, close: () => ws.close() };
}

async function main() {
    const home = path.join(tmp, "home");
    const data = path.join(tmp, "data");
    fs.mkdirSync(home, { recursive: true });
    fs.mkdirSync(data, { recursive: true });
    const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith("AGENTMUX_")));
    env.AGENTMUX_HOME_OVERRIDE = home;
    const srv = spawn(srvExe, ["--headless", "--frontend-dir", webDir, "--wavedata", data, "--web-port", String(port), "--ws-port", String(port + 1)], { env, stdio: "ignore" });
    children.push(srv);
    const base = `http://127.0.0.1:${port}`;
    for (let i = 0; i < 120; i++) {
        try {
            if ((await fetch(`${base}/health`)).ok) break;
        } catch {
            await sleep(500);
        }
    }
    const key = fs.readFileSync(path.join(data, "srv-auth-key"), "utf8").trim();
    const ask = await signIn(base, key);
    const { code } = await ask("/auth/code", {});

    const cdpPort = port + 20;
    const chrome = spawn(chromeExe, ["--headless=new", `--remote-debugging-port=${cdpPort}`, `--user-data-dir=${path.join(tmp, "chrome")}`, "--window-size=1400,950", "--no-first-run", "--disable-gpu", "about:blank"], { stdio: "ignore" });
    children.push(chrome);
    const page = await attach(cdpPort);
    for (const m of ["Runtime.enable", "Log.enable", "Network.enable", "Page.enable"]) await page.call(m);
    // Every policy violation the page itself sees, from its first script on.
    await page.call("Page.addScriptToEvaluateOnNewDocument", {
        source: `window.__violations = []; document.addEventListener("securitypolicyviolation", (e) => window.__violations.push(e.effectiveDirective + " " + e.blockedURI + " " + (e.sourceFile || "") + ":" + (e.lineNumber || "")));`,
    });
    // `allowUnsafeEvalBlockedByCSP: false`: DevTools' own evaluation is exempt from the page's policy unless told not to be.
    const evaluate = async (expression) =>
        (await page.call("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true, allowUnsafeEvalBlockedByCSP: false })).result?.result?.value;

    await page.call("Page.navigate", { url: `${base}/auth/enter#${code}` });
    // The page shows the check phrase once the code is entered; the "person" answers yes.
    for (let i = 0; i < 60; i++) {
        const state = await ask("/auth/pending", { code });
        if (state.state === "entered") break;
        await sleep(300);
    }
    await ask("/auth/confirm", { code, accept: true });
    // Signed in: the page hands over to the UI.
    let loaded = false;
    for (let i = 0; i < 120 && !loaded; i++) {
        await sleep(500);
        loaded = (await evaluate(`!!document.querySelector("#main") && document.querySelector("#main").children.length > 0 && !document.querySelector("#startup-loading")`)) === true;
    }
    await sleep(3000);

    // A few panes from the command palette (Ctrl+P, the command's name, Enter).
    const run = async (label) => {
        await page.call("Input.dispatchKeyEvent", { type: "keyDown", key: "p", code: "KeyP", windowsVirtualKeyCode: 80, modifiers: 2 });
        await page.call("Input.dispatchKeyEvent", { type: "keyUp", key: "p", code: "KeyP", windowsVirtualKeyCode: 80, modifiers: 2 });
        await sleep(500);
        await page.call("Input.insertText", { text: label });
        await sleep(400);
        for (const type of ["keyDown", "keyUp"]) await page.call("Input.dispatchKeyEvent", { type, key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, text: type === "keyDown" ? "\r" : undefined });
        await sleep(2500);
    };
    for (const label of ["Open Terminal", "Open System Info"]) await run(label);

    const violations = (await evaluate("window.__violations")) ?? [];
    // What the policy is for: WebAssembly compiles; text is not evaluated.
    const wasm = await evaluate(`WebAssembly.compile(new Uint8Array([0,97,115,109,1,0,0,0])).then(() => "compiles", (e) => "refused: " + e.message)`);
    const evalProbe = await evaluate(`(() => { try { return (0, eval)("1"); } catch (e) { return "refused"; } })()`);
    const errors = page.events
        .filter((e) => (e.method === "Runtime.consoleAPICalled" && e.params.type === "error") || e.method === "Runtime.exceptionThrown" || (e.method === "Log.entryAdded" && ["error", "warning"].includes(e.params.entry.level)))
        .map((e) => JSON.stringify(e.params).slice(0, 300));
    const failed = page.events.filter((e) => e.method === "Network.loadingFailed").map((e) => `${e.params.errorText} ${e.params.blockedReason ?? ""}`);
    page.close();

    console.log(`ui loaded: ${loaded}`);
    console.log(`WebAssembly: ${wasm} (want compiles)`);
    console.log(`eval of text: ${evalProbe === "refused" ? "refused" : "ALLOWED"} (want refused)`);
    console.log(`policy violations before the probes: ${violations.length}`);
    for (const v of violations) console.log(`  ${v}`);
    console.log(`console errors and warnings: ${errors.length}`);
    for (const e of errors.slice(0, 20)) console.log(`  ${e}`);
    console.log(`failed requests: ${failed.length}`);
    for (const f of failed.slice(0, 20)) console.log(`  ${f}`);
    return loaded && violations.length === 0 && wasm === "compiles" && evalProbe === "refused";
}

let ok = false;
try {
    ok = await main();
} catch (e) {
    console.error(`headless-ui-check: ${e.message}`);
} finally {
    for (const c of children) {
        try {
            c.kill();
        } catch {
            /* already gone */
        }
    }
    await sleep(1500);
    fs.rmSync(tmp, { recursive: true, force: true });
}
console.log(ok ? "OK: a clean load reports no policy violation" : "FAILED");
process.exit(ok ? 0 : 1);
