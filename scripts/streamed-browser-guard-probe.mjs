#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// streamed-browser-guard-probe.mjs: checks, with the real Chromium of a headless srv, that a page cannot reach what the
// address proxy refuses (docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md section 12).
//
// It serves a page on 127.0.0.1, opens a pane on it over the WebSocket (as streamed-browser-probe.mjs does), and the
// page tries to connect to: the cloud metadata addresses, private ranges, and srv's own ports by address, by
// `localhost`, by IPv6 and by a public name that resolves to loopback. Each must fail, and fail at once (the proxy
// refuses; a connection that merely timed out is not the proxy's answer). A connection to the probe's own server
// (another loopback port, like a development server) must succeed. Prints JSON; exits 1 on any violation.
//
//   node scripts/streamed-browser-guard-probe.mjs --port 8190 --ws-port 8191 --key-file <path> [--extra-denied 8080]
//
// Run it in the same container as srv. Needs the `ws` package. The by-name case needs DNS (`localtest.me`).

import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import WebSocket from "ws";

const args = process.argv.slice(2);
const opt = (name, fallback) => {
    const i = args.indexOf(`--${name}`);
    return i >= 0 ? args[i + 1] : fallback;
};
const webPort = Number(opt("port", "8190"));
const wsPort = Number(opt("ws-port", "8191"));
const extra = opt("extra-denied") ? Number(opt("extra-denied")) : null;
const keyFile = opt("key-file");
if (!keyFile) {
    console.error("usage: streamed-browser-guard-probe.mjs --port <web> --ws-port <ws> --key-file <path> [--extra-denied <port>]");
    process.exit(2);
}
const key = fs.readFileSync(keyFile, "utf8").trim();

// The addresses this machine has on its own network (a container's private address): the page must not reach them.
function ownLanAddresses() {
    const out = [];
    for (const list of Object.values(os.networkInterfaces())) {
        for (const i of list ?? []) if (i.family === "IPv4" && !i.internal) out.push(i.address);
    }
    return out;
}

// What the page tries. `reach: true` must connect; the rest must be refused quickly.
const CASES = [
    { label: "own-server (a development server on another port)", url: "ORIGIN/ping", reach: true },
    { label: "task-metadata (169.254.170.2)", url: "http://169.254.170.2/v4/x" },
    { label: "instance-metadata (169.254.169.254)", url: "http://169.254.169.254/latest/meta-data/" },
    { label: "carrier-grade NAT 100.64.0.1", url: "http://100.64.0.1/" },
    { label: "benchmark range 198.18.0.1", url: "http://198.18.0.1/" },
    ...ownLanAddresses().map((a) => ({ label: `this machine on its own network address (${a})`, url: `http://${a}/` })),
    { label: "srv web port by address", url: `http://127.0.0.1:${webPort}/health` },
    { label: "srv ws port by localhost", url: `http://localhost:${wsPort}/` },
    { label: "srv web port by IPv6", url: `http://[::1]:${webPort}/health` },
    { label: "srv web port by a public name that resolves to loopback", url: `http://localtest.me:${webPort}/health` },
    ...(extra ? [{ label: `extra denied port ${extra}`, url: `http://127.0.0.1:${extra}/` }] : []),
];

const page = (origin) => `<!doctype html><meta charset=utf-8><title>guard</title><body>guard probe
<script>
const cases = ${JSON.stringify(CASES)}.map((c) => ({ ...c, url: c.url.replace('ORIGIN', '${origin}') }));
(async () => {
  const out = {};
  await Promise.all(cases.map(async (c) => {
    const t0 = performance.now();
    try {
      await fetch(c.url, { mode: 'no-cors', cache: 'no-store', signal: AbortSignal.timeout(12000) });
      out[c.label] = { reached: true, ms: Math.round(performance.now() - t0) };
    } catch (e) {
      out[c.label] = { reached: false, ms: Math.round(performance.now() - t0), err: String(e) };
    }
  }));
  // WebRTC may not send a packet anywhere but through the proxy: with a STUN server named, no server-reflexive
  // candidate (one learned from outside) and no candidate with a literal address may come out.
  try {
    const pc = new RTCPeerConnection({ iceServers: [{ urls: 'stun:stun.l.google.com:19302' }] });
    pc.createDataChannel('x');
    const cands = [];
    pc.onicecandidate = (e) => { if (e.candidate) cands.push(e.candidate.candidate); };
    await pc.setLocalDescription(await pc.createOffer());
    await new Promise((r) => { pc.onicegatheringstatechange = () => pc.iceGatheringState === 'complete' && r(); setTimeout(r, 7000); });
    pc.close();
    out['webrtc'] = { candidates: cands };
  } catch (e) {
    out['webrtc'] = { candidates: [], err: String(e) };
  }
  navigator.sendBeacon('/report', JSON.stringify(out));
})();
</script>`;

let resolveReport;
const reported = new Promise((r) => (resolveReport = r));
const server = http.createServer((req, res) => {
    if (req.url === "/report") {
        let body = "";
        req.on("data", (d) => (body += d));
        req.on("end", () => {
            res.writeHead(204).end();
            resolveReport(JSON.parse(body));
        });
        return;
    }
    if (req.url === "/ping") return void res.writeHead(200, { "access-control-allow-origin": "*" }).end("ok");
    res.writeHead(200, { "content-type": "text/html" }).end(page(`http://127.0.0.1:${server.address().port}`));
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const probePort = server.address().port;

const ws = new WebSocket(
    `ws://127.0.0.1:${webPort}/agentmux/streamed-browser?url=${encodeURIComponent(`http://127.0.0.1:${probePort}/`)}&w=800&h=500`,
    { headers: { "X-AuthKey": key } },
);
const errors = [];
ws.on("message", (data, isBinary) => {
    if (!isBinary) {
        const m = JSON.parse(data.toString());
        if (m.t === "error") errors.push(m.message);
    }
});
ws.on("error", (e) => errors.push(String(e)));

const report = await Promise.race([reported, new Promise((r) => setTimeout(() => r(null), 40000))]);
ws.close();
server.close();

const violations = [];
if (!report) violations.push(`no report from the page${errors.length ? `: ${errors.join("; ")}` : ""}`);
else {
    for (const c of CASES) {
        const r = report[c.label];
        if (!r) violations.push(`${c.label}: no result`);
        else if (c.reach && !r.reached) violations.push(`${c.label}: should connect, did not (${r.err})`);
        else if (!c.reach && r.reached) violations.push(`${c.label}: REACHED, must be refused`);
        else if (!c.reach && r.ms > 3000) violations.push(`${c.label}: failed only after ${r.ms} ms (a timeout, not the proxy's refusal)`);
    }
}
// A candidate is "typ host" with an address, or "srflx"/"relay"; a name ending in .local (mDNS) leaks no address.
const rtc = report?.webrtc;
if (report && !rtc) violations.push("webrtc: no result");
for (const c of rtc?.candidates ?? []) {
    const literal = /candidate:\S+ \d+ \S+ \d+ (\S+) \d+ typ (\S+)/.exec(c);
    if (literal && (literal[2] !== "host" || !literal[1].endsWith(".local"))) violations.push(`webrtc: candidate leaves the proxy: ${c}`);
}
console.log(JSON.stringify({ ok: violations.length === 0, violations, report }, null, 2));
process.exit(violations.length ? 1 : 0);
