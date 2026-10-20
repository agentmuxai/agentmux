#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A local HTTP/HTTPS (CONNECT) proxy that makes the network refuse, blackhole,
 * fail name resolution, or flap, switchable at run time, for checking how a
 * dev instance and its agent CLIs behave offline. Point them at it with
 * `HTTPS_PROXY`/`HTTP_PROXY`; switch modes with `ctl.mjs` or `POST /mode` on the
 * control port. What each mode looks like to a client: README.md.
 *
 * Part of the fault injection in docs/specs/SPEC_OFFLINE_RESILIENCE_2026_10_20.md §7.1.
 *
 *   node scripts/net-fault/proxy.mjs [--port 8899] [--control-port 8898] [--mode pass]
 */

import http from "node:http";
import net from "node:net";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import { clearRules, createState, decide, describe, nextChangeIn, refusesEverything, setMode } from "./modes.mjs";

export const DEFAULT_PORT = 8899;
export const DEFAULT_CONTROL_PORT = 8898;

/** Local wall-clock time, to line up with what the window shows. */
function stamp() {
    const d = new Date();
    return `${d.toTimeString().slice(0, 8)}.${String(d.getMilliseconds()).padStart(3, "0")}`;
}

/** `host:port` or `[v6]:port` from a CONNECT target. */
export function splitHostPort(target, defaultPort = 443) {
    const m = /^\[([^\]]+)\](?::(\d+))?$/.exec(target) ?? /^([^:]+)(?::(\d+))?$/.exec(target);
    if (!m) return null;
    return { host: m[1].toLowerCase(), port: Number(m[2] ?? defaultPort) };
}

/** The 502 a CONNECT gets in `dns` mode; the body is what Node prints for a failed lookup. */
function dnsFailure(host) {
    const body = `getaddrinfo ENOTFOUND ${host}\n`;
    return `HTTP/1.1 502 Bad Gateway\r\nX-Net-Fault: dns\r\nContent-Type: text/plain\r\nContent-Length: ${body.length}\r\nConnection: close\r\n\r\n${body}`;
}

/** RST, not FIN, so the client sees `ECONNRESET` rather than a clean close. */
function reset(socket) {
    if (socket.destroyed) return;
    if (typeof socket.resetAndDestroy === "function" && socket._handle) socket.resetAndDestroy();
    else socket.destroy();
}

export async function startProxy({
    port = DEFAULT_PORT,
    controlPort = DEFAULT_CONTROL_PORT,
    bind = "127.0.0.1",
    mode = "pass",
    log = (line) => console.log(line),
} = {}) {
    const state = createState(mode);
    /** Every CONNECT not yet closed: `pending` (no answer yet), `connecting`, `open`, or `frozen`. */
    const conns = new Set();
    /** Plain-HTTP requests held by `blackhole`, answered at the next mode change or ended by `close()`. */
    const hanging = new Set();
    let listening = false;
    let timer = null;
    let lastSummary = "";
    let queue = Promise.resolve();

    const say = (conn, mode, outcome) => log(`${stamp()} CONNECT ${conn.host}:${conn.port} [${mode}] -> ${outcome}`);

    function pipeBoth(conn) {
        conn.client.pipe(conn.upstream);
        conn.upstream.pipe(conn.client);
        conn.status = "open";
    }

    function openTunnel(conn) {
        conn.status = "connecting";
        const up = net.connect(conn.port, conn.host);
        conn.upstream = up;
        up.once("connect", () => {
            if (conn.client.destroyed) return up.destroy();
            // The mode may have changed while the connect was in flight.
            const now = decide(state, conn.host);
            if (now !== "pass") {
                up.destroy();
                conn.upstream = null;
                conn.status = "pending";
                return act(conn, now);
            }
            conn.client.write("HTTP/1.1 200 Connection Established\r\n\r\n");
            if (conn.head?.length) up.write(conn.head);
            pipeBoth(conn);
            say(conn, "pass", "tunnel open");
        });
        up.on("error", (err) => {
            // While frozen the client must notice nothing; "close" records it for the thaw.
            if (conn.status === "frozen") return;
            if (conn.status !== "connecting") return conn.client.destroy();
            conn.status = "failed";
            // A real failure upstream is passed on in the same shape the fault modes use.
            if (err.code === "ENOTFOUND" || err.code === "EAI_AGAIN") conn.client.end(dnsFailure(conn.host));
            else reset(conn.client);
            say(conn, "pass", `upstream ${err.code ?? err.message}`);
        });
        up.on("close", (hadError) => {
            if (conn.status === "frozen") conn.upstreamGone = hadError ? "reset" : "closed";
            else if (hadError) conn.client.destroy();
        });
    }

    function freeze(conn) {
        conn.client.unpipe(conn.upstream);
        conn.upstream.unpipe(conn.client);
        conn.client.pause();
        conn.upstream.pause();
        conn.status = "frozen";
    }

    function cut(conn) {
        reset(conn.client);
        conn.upstream?.destroy();
        conns.delete(conn);
    }

    /** Applies `mode` to one CONNECT, whatever stage it is at. */
    function act(conn, mode) {
        conn.applied = mode;
        const { status } = conn;
        if (mode === "pass") {
            if (status === "pending") openTunnel(conn);
            else if (status === "frozen" && conn.upstreamGone) {
                // The far end went away while frozen; the client learns it only now.
                if (conn.upstreamGone === "reset") reset(conn.client);
                else conn.client.end();
                conns.delete(conn);
                say(conn, mode, `thawed, upstream had ${conn.upstreamGone === "reset" ? "reset" : "closed"}`);
            } else if (status === "frozen") {
                pipeBoth(conn);
                say(conn, mode, "thawed");
            }
        } else if (mode === "refuse") {
            cut(conn);
            say(conn, mode, status === "pending" ? "reset" : "cut (reset)");
        } else if (mode === "dns") {
            if (status === "pending") {
                conn.client.end(dnsFailure(conn.host));
                conns.delete(conn);
                say(conn, mode, "502 ENOTFOUND");
            } else {
                // Inside an open tunnel there is no HTTP left to answer with.
                cut(conn);
                say(conn, mode, "cut (reset)");
            }
        } else if (mode === "blackhole") {
            if (status === "pending") say(conn, mode, "no answer");
            else if (status === "open") {
                freeze(conn);
                say(conn, mode, "frozen");
            }
        }
    }

    function onConnect(req, client, head) {
        client.on("error", () => {});
        const target = splitHostPort(req.url);
        if (!target) return client.end("HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n");
        const conn = { ...target, client, upstream: null, status: "pending", head, applied: null };
        conns.add(conn);
        client.on("close", () => {
            conns.delete(conn);
            conn.upstream?.destroy();
        });
        act(conn, decide(state, conn.host));
    }

    /** Plain `http://` through `HTTP_PROXY`: the same decisions, without tunnels to manage. */
    function onRequest(req, res) {
        let url;
        try {
            url = new URL(req.url);
        } catch {
            return res
                .writeHead(400, { "content-type": "text/plain" })
                .end("net-fault: this port is the proxy; the control API is on the control port\n");
        }
        const held = { req, res, url };
        res.on("close", () => hanging.delete(held));
        answer(held, decide(state, url.hostname));
    }

    /** Answers one plain-HTTP request in `mode`; under `blackhole` it waits in `hanging`. */
    function answer(held, mode) {
        const { req, res, url } = held;
        log(
            `${stamp()} ${req.method} ${url.host} [${mode}] -> ${{ pass: "forwarded", refuse: "reset", dns: "502 ENOTFOUND", blackhole: "no answer" }[mode]}`
        );
        if (mode === "refuse") return reset(req.socket);
        if (mode === "dns")
            return res.writeHead(502, { "x-net-fault": "dns" }).end(`getaddrinfo ENOTFOUND ${url.hostname}\n`);
        if (mode === "blackhole") return void hanging.add(held);
        const up = http.request(url, { method: req.method, headers: req.headers }, (ur) => {
            res.writeHead(ur.statusCode ?? 502, ur.headers);
            ur.pipe(res);
        });
        up.on("error", (err) =>
            res.headersSent ? res.destroy() : res.writeHead(502).end(`${err.code ?? err.message}\n`)
        );
        req.pipe(up);
    }

    const proxy = http.createServer(onRequest);
    proxy.on("connect", onConnect);
    proxy.on("clientError", (_err, socket) => socket.destroy());
    let actualPort = port;

    function listen() {
        return new Promise((resolve, reject) => {
            proxy.once("error", reject);
            proxy.listen(actualPort, bind, () => {
                proxy.off("error", reject);
                actualPort = proxy.address().port;
                listening = true;
                resolve();
            });
        });
    }

    function unlisten() {
        listening = false;
        proxy.close();
    }

    async function reevaluate() {
        const now = Date.now();
        try {
            const summary = JSON.stringify(describe(state, now));
            if (summary !== lastSummary) {
                const d = describe(state, now);
                const rules = d.rules.map((r) => ` ${r.host}=${r.now}${r.mode === r.now ? "" : ` (${r.mode})`}`).join("");
                log(
                    `${stamp()} now: default=${d.default.now}${d.default.mode === d.default.now ? "" : ` (${d.default.mode})`}${rules}`
                );
                lastSummary = summary;
            }
            for (const conn of [...conns]) {
                const mode = decide(state, conn.host, now);
                if (mode !== conn.applied) act(conn, mode);
            }
            for (const held of [...hanging]) {
                const mode = decide(state, held.url.hostname, now);
                if (mode !== "blackhole") {
                    hanging.delete(held);
                    answer(held, mode);
                }
            }
            const wantListen = !refusesEverything(state, now);
            if (wantListen && !listening) {
                await listen();
                log(`${stamp()} listening again on ${bind}:${actualPort}`);
            } else if (!wantListen && listening) {
                unlisten();
                log(`${stamp()} every host refused: stopped listening, so new connects get ECONNREFUSED`);
            }
        } finally {
            // Re-arm even when listen() failed, so flapping carries on.
            clearTimeout(timer);
            const next = nextChangeIn(state, now);
            // A few ms past the boundary, so the phase has actually flipped when this runs.
            if (next !== null) timer = setTimeout(scheduleFromTimer, next + 5);
        }
    }

    function schedule() {
        queue = queue.then(reevaluate, reevaluate);
        return queue;
    }

    // Nothing awaits a timer's run, so log its failure instead of letting it end the proxy.
    function scheduleFromTimer() {
        schedule().catch((err) => log(`${stamp()} re-evaluating failed: ${err.message}`));
    }

    const status = () => ({
        proxy: `http://${bind}:${actualPort}`,
        listening,
        ...describe(state),
        connections: conns.size,
    });

    async function readBody(req) {
        let text = "";
        for await (const chunk of req) text += chunk;
        return text.trim() ? JSON.parse(text) : {};
    }

    const control = http.createServer(async (req, res) => {
        const url = new URL(req.url, "http://control");
        const send = (code, body) =>
            res.writeHead(code, { "content-type": "application/json" }).end(`${JSON.stringify(body, null, 2)}\n`);
        try {
            if (req.method === "GET" && url.pathname === "/status") return send(200, status());
            if (req.method === "POST" && url.pathname === "/mode") {
                const body = await readBody(req);
                const mode = body.mode ?? url.searchParams.get("mode");
                const host = body.host ?? url.searchParams.get("host") ?? undefined;
                await api.setMode(mode, host);
                return send(200, status());
            }
            if (req.method === "POST" && url.pathname === "/clear") {
                await api.clear();
                return send(200, status());
            }
            send(404, { error: "GET /status, POST /mode {mode, host?}, POST /clear" });
        } catch (err) {
            send(400, { error: err.message });
        }
    });

    await listen();
    await new Promise((resolve, reject) => {
        control.once("error", reject);
        control.listen(controlPort, bind, resolve);
    });
    await schedule();

    const api = {
        get port() {
            return actualPort;
        },
        controlPort: control.address().port,
        state,
        status,
        setMode(modeText, host) {
            if (!modeText) throw new Error("mode is required");
            setMode(state, modeText, host);
            log(`${stamp()} set ${host ? `rule ${host}` : "default"} = ${modeText}`);
            return schedule();
        },
        clear() {
            clearRules(state);
            log(`${stamp()} cleared every host rule`);
            return schedule();
        },
        async close() {
            clearTimeout(timer);
            for (const conn of [...conns]) cut(conn);
            for (const { req } of hanging) req.socket.destroy();
            control.close();
            if (listening) unlisten();
        },
    };
    return api;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
    const { values } = parseArgs({
        options: {
            port: { type: "string", default: String(DEFAULT_PORT) },
            "control-port": { type: "string", default: String(DEFAULT_CONTROL_PORT) },
            mode: { type: "string", default: "pass" },
        },
    });
    const p = await startProxy({
        port: Number(values.port),
        controlPort: Number(values["control-port"]),
        mode: values.mode,
    });
    console.log(`net-fault proxy on http://127.0.0.1:${p.port}, control on http://127.0.0.1:${p.controlPort}`);
    console.log(
        `use: HTTPS_PROXY=http://127.0.0.1:${p.port} HTTP_PROXY=http://127.0.0.1:${p.port} NO_PROXY=127.0.0.1,localhost,::1`
    );
    process.on("SIGINT", async () => {
        await p.close();
        process.exit(0);
    });
}
