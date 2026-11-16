#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// streamed-browser-probe.mjs: drives a headless srv's streamed Browser pane the way the web UI does, with no UI.
// docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md §9.
//
// It serves a small test page on 127.0.0.1 (so the server's Chromium must be able to reach this machine's
// loopback: run the probe in the same container as srv), opens a pane over the WebSocket with srv's key, and measures,
// each in milliseconds from the message sent to the page's own report (or to the frame):
//   first frame, click, key, wheel, paste, and resize (to the page's resize event and to a frame of the new size).
//
//   node scripts/streamed-browser-probe.mjs --port 8190 --key-file <data dir>/srv-auth-key [--runs 5] [--json]
//
// Needs the `ws` package (a dev dependency of the repo). The key never leaves this machine's loopback.

import fs from "node:fs";
import http from "node:http";
import WebSocket from "ws";

const args = process.argv.slice(2);
const opt = (name, fallback) => {
    const i = args.indexOf(`--${name}`);
    return i >= 0 ? args[i + 1] : fallback;
};
const port = Number(opt("port", "8190"));
const keyFile = opt("key-file");
const runs = Number(opt("runs", "5"));
const asJson = args.includes("--json");
if (!keyFile) {
    console.error("usage: streamed-browser-probe.mjs --port <web port> --key-file <path> [--runs N] [--json]");
    process.exit(2);
}
const key = fs.readFileSync(keyFile, "utf8").trim();

const PAGE = `<!doctype html><meta charset=utf-8><title>probe</title>
<style>body{margin:0;font:16px sans-serif}#a{position:absolute;left:100px;top:100px;width:200px;height:80px;background:#48c}
#t{height:3000px}input{position:fixed;left:100px;top:300px;width:300px}</style>
<div id=a></div><button id=p style="position:fixed;left:400px;top:100px;width:120px;height:40px">popup</button><input id=f type=file style="position:fixed;left:600px;top:100px;width:200px;height:40px"><input id=i autofocus><div id=t></div>
<script>
const send = (k, v) => navigator.sendBeacon('/e?k=' + k + '&v=' + encodeURIComponent(v) + '&n=' + performance.now());
document.getElementById('a').addEventListener('mousedown', (e) => send('click', e.clientX + ',' + e.clientY));
document.getElementById('p').addEventListener('click', () => {
    const w = window.open('/popup', '_blank');
    const timer = setInterval(() => { if (!w || w.closed) { send('popup-closed', '1'); clearInterval(timer); } }, 50);
});
document.getElementById('f').addEventListener('click', () => send('file-click', '1'));
document.getElementById('f').addEventListener('change', () => send('file-change', '1'));
addEventListener('keydown', (e) => send('key', e.key));
addEventListener('scroll', () => send('wheel', scrollY));
document.getElementById('i').addEventListener('input', (e) => send('paste', e.target.value));
addEventListener('resize', () => send('resize', innerWidth + 'x' + innerHeight));
addEventListener('load', () => send('ready', innerWidth + 'x' + innerHeight));
</script>`;

const reports = [];
const waiters = [];
const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://x");
    if (url.pathname === "/e") {
        const r = { k: url.searchParams.get("k"), v: url.searchParams.get("v"), at: performance.now() };
        reports.push(r);
        for (const w of waiters.splice(0)) w(r);
        res.writeHead(204).end();
        return;
    }
    res.writeHead(200, { "content-type": "text/html" }).end(PAGE);
});

const until = (pred, ms = 8000) =>
    new Promise((resolve, reject) => {
        const found = reports.find(pred);
        if (found) return resolve(found);
        const t = setTimeout(() => reject(new Error("timed out")), ms);
        const check = (r) => (pred(r) ? (clearTimeout(t), resolve(r)) : waiters.push(check));
        waiters.push(check);
    });

/** The width and height in a JPEG's start-of-frame marker. */
function jpegSize(buf) {
    for (let i = 3; i + 9 < buf.length; ) {
        if (buf[i] !== 0xff) { i++; continue; }
        const m = buf[i + 1];
        if (m >= 0xc0 && m <= 0xcf && m !== 0xc4 && m !== 0xc8 && m !== 0xcc) return { h: buf.readUInt16BE(i + 5), w: buf.readUInt16BE(i + 7) };
        i += 2 + buf.readUInt16BE(i + 2);
    }
    return null;
}

function open(pageUrl, w, h) {
    const frames = [];
    const frameWaiters = [];
    const ws = new WebSocket(`ws://127.0.0.1:${port}/agentmux/streamed-browser?url=${encodeURIComponent(pageUrl)}&w=${w}&h=${h}`, { headers: { "X-AuthKey": key } });
    const errors = [];
    ws.on("message", (data, isBinary) => {
        if (isBinary) {
            const size = jpegSize(data.subarray(1));
            const f = { at: performance.now(), bytes: data.length - 1, ...size };
            frames.push(f);
            for (const fw of frameWaiters.splice(0)) fw(f);
        } else {
            const m = JSON.parse(data.toString());
            if (m.t === "error") errors.push(m.message);
        }
    });
    const frame = (pred, ms = 8000) =>
        new Promise((resolve, reject) => {
            const found = frames.find(pred);
            if (found) return resolve(found);
            const t = setTimeout(() => reject(new Error("no frame")), ms);
            const check = (f) => (pred(f) ? (clearTimeout(t), resolve(f)) : frameWaiters.push(check));
            frameWaiters.push(check);
        });
    const opened = new Promise((resolve, reject) => { ws.on("open", resolve); ws.on("error", reject); });
    return { ws, frames, errors, frame, opened, send: (m) => ws.send(JSON.stringify(m)) };
}

const median = (xs) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];
const result = { file_input_ok: false, cold_ms: 0, popup_closed_ms: [], first_frame_ms: [], click_ms: [], key_ms: [], wheel_ms: [], paste_ms: [], resize: {} };

async function oneRun(base) {
    const t0 = performance.now();
    const pane = open(`${base}/`, 1024, 640);
    await pane.opened;
    const first = await pane.frame(() => true);
    result.first_frame_ms.push(first.at - t0);
    await until((r) => r.k === "ready" && r.at > t0);

    let t = performance.now();
    pane.send({ t: "mouse", type: "move", x: 150, y: 130 });
    pane.send({ t: "mouse", type: "down", x: 150, y: 130, button: "left", buttons: 1, clicks: 1 });
    pane.send({ t: "mouse", type: "up", x: 150, y: 130, button: "left", buttons: 0, clicks: 1 });
    const click = await until((r) => r.k === "click" && r.at > t);
    if (click.v !== "150,130") throw new Error(`click arrived at ${click.v}`);
    result.click_ms.push(click.at - t);

    t = performance.now();
    pane.send({ t: "key", type: "down", key: "Shift", code: "ShiftLeft", vk: 16 });
    pane.send({ t: "key", type: "up", key: "Shift", code: "ShiftLeft", vk: 16 });
    const keyed = await until((r) => r.k === "key" && r.at > t);
    result.key_ms.push(keyed.at - t);

    t = performance.now();
    pane.send({ t: "wheel", x: 500, y: 400, dx: 0, dy: 400 });
    const scrolled = await until((r) => r.k === "wheel" && r.at > t && Number(r.v) > 0);
    result.wheel_ms.push(scrolled.at - t);

    // The input is fixed in the viewport (the wheel step has scrolled the page): a click focuses it, a paste goes in as text.
    t = performance.now();
    pane.send({ t: "mouse", type: "down", x: 200, y: 310, button: "left", buttons: 1, clicks: 1 });
    pane.send({ t: "mouse", type: "up", x: 200, y: 310, button: "left", buttons: 0, clicks: 1 });
    pane.send({ t: "paste", text: "pasted-text" });
    const pasted = await until((r) => r.k === "paste" && r.at > t);
    if (pasted.v !== "pasted-text") throw new Error(`paste arrived as ${pasted.v}`);
    result.paste_ms.push(pasted.at - t);

    // A window the page opens is closed by the server: the page sees it closed.
    t = performance.now();
    pane.send({ t: "mouse", type: "down", x: 460, y: 120, button: "left", buttons: 1, clicks: 1 });
    pane.send({ t: "mouse", type: "up", x: 460, y: 120, button: "left", buttons: 0, clicks: 1 });
    const popup = await until((r) => r.k === "popup-closed" && r.at > t);
    result.popup_closed_ms.push(popup.at - t);

    // A click on a file input opens no chooser: the page sees its click and never a chosen file, and the pane goes on working.
    t = performance.now();
    pane.send({ t: "mouse", type: "down", x: 650, y: 120, button: "left", buttons: 1, clicks: 1 });
    pane.send({ t: "mouse", type: "up", x: 650, y: 120, button: "left", buttons: 0, clicks: 1 });
    await until((r) => r.k === "file-click" && r.at > t);
    await new Promise((r) => setTimeout(r, 600));
    if (reports.some((r) => r.k === "file-change" && r.at > t)) throw new Error("a file was chosen on the server");
    t = performance.now();
    pane.send({ t: "key", type: "down", key: "Shift", code: "ShiftLeft", vk: 16 });
    await until((r) => r.k === "key" && r.at > t);
    result.file_input_ok = true;

    for (const [w, h] of [[800, 600], [1024, 640], [1600, 900]]) {
        const sent = performance.now();
        pane.send({ t: "resize", w, h });
        const sized = await until((r) => r.k === "resize" && r.at > sent && r.v === `${w}x${h}`, 5000).catch(() => null);
        const frame = await pane.frame((f) => f.at > sent && f.w === w && f.h === h, 5000).catch(() => null);
        const slot = (result.resize[`${w}x${h}`] ??= { page_ms: [], frame_ms: [], missed: 0 });
        if (sized) slot.page_ms.push(sized.at - sent); else slot.missed++;
        if (frame) slot.frame_ms.push(frame.at - sent); else slot.missed++;
    }
    if (pane.errors.length) throw new Error(`the pane said: ${pane.errors.join("; ")}`);
    pane.ws.close();
    // Let the server close the page before the next run.
    await new Promise((r) => setTimeout(r, 300));
}

await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}`;
let failed = null;
let concurrent = null;
try {
    // Two panes asked for at the same moment, with no browser running yet: both must get a frame (one Chromium starts).
    const started = performance.now();
    const two = [open(`${base}/`, 800, 600), open(`${base}/`, 640, 480)];
    await Promise.all(two.map((p) => p.opened));
    const firsts = await Promise.all(two.map((p) => p.frame(() => true)));
    concurrent = firsts.length === 2;
    // This is the cold start: no Chromium was running before these two panes.
    result.cold_ms = Math.max(...firsts.map((f) => f.at)) - started;
    for (const p of two) p.ws.close();
    await new Promise((r) => setTimeout(r, 300));
    for (let i = 0; i < runs; i++) await oneRun(base);
} catch (e) {
    failed = e;
}
server.close();

const summary = {
    runs,
    // The two panes asked for together start the server's Chromium; the median of the runs after it is the warm figure.
    first_frame_cold_ms: Math.round(result.cold_ms),
    first_frame_ms: Math.round(median(result.first_frame_ms) || 0),
    click_ms: Math.round(median(result.click_ms) || 0),
    key_ms: Math.round(median(result.key_ms) || 0),
    wheel_ms: Math.round(median(result.wheel_ms) || 0),
    paste_ms: Math.round(median(result.paste_ms) || 0),
    popup_closed_ms: Math.round(median(result.popup_closed_ms) || 0),
    two_panes_at_once: concurrent,
    file_input_ok: result.file_input_ok,
    resize: Object.fromEntries(
        Object.entries(result.resize).map(([k, v]) => [k, { page_ms: Math.round(median(v.page_ms) || 0), frame_ms: Math.round(median(v.frame_ms) || 0), missed: v.missed }])
    ),
    error: failed ? String(failed.message ?? failed) : null,
};
console.log(asJson ? JSON.stringify(summary) : JSON.stringify(summary, null, 2));
process.exit(failed ? 1 : 0);
