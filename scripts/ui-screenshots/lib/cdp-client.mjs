// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Minimal Chrome DevTools Protocol client for the measurement scripts in
// scripts/ui-screenshots/. One implementation instead of one per script, with
// the failure handling Codex drove into the per-script clients on #3569:
//
//   - an error response rejects (a failed Tracing.start must not look like a
//     success and leave the caller waiting forever);
//   - a closed or errored socket rejects everything still pending, and any
//     later send, instead of hanging;
//   - a send that fails to write rejects;
//   - every call has a timeout.
//
// Used by full-conversation-bench.mjs
// (docs/specs/SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md, Phase 0).

import { createRequire } from "node:module";
import { dirname, join } from "node:path";

/**
 * The Node build of `ws`, whoever runs this module. Under vitest (this repo's
 * config runs workers with the `browser` resolve condition) both
 * `import "ws"` and `require("ws")` resolve through ws's exports map to
 * `browser.js`, a stub with no client or server. Loading `index.js` from the
 * package directory bypasses condition resolution; `./package.json` is an
 * exported subpath, so locating the directory is allowed.
 */
export function nodeWs() {
    const req = createRequire(import.meta.url);
    return req(join(dirname(req.resolve("ws/package.json")), "index.js"));
}

const WebSocket = nodeWs();

/** The production AgentMux instance listens on 9222; dev builds on 9223. */
export const PRODUCTION_CDP_PORT = 9222;

export class CdpError extends Error {}

/**
 * Wrap an open WebSocket as a CDP session.
 * @param {WebSocket} ws
 * @param {{ timeoutMs?: number }} [opts]
 */
export function cdpSession(ws, { timeoutMs = 120_000 } = {}) {
    let nextId = 0;
    let closedReason = null;
    const pending = new Map();
    const listeners = new Map();

    const failAll = (why) => {
        if (closedReason === null) closedReason = why;
        for (const { reject, timer } of pending.values()) {
            clearTimeout(timer);
            reject(new CdpError(why));
        }
        pending.clear();
    };

    ws.on("message", (raw) => {
        let msg;
        try {
            msg = JSON.parse(raw);
        } catch {
            return;
        }
        if (msg.id !== undefined && pending.has(msg.id)) {
            const { resolve, reject, timer, method } = pending.get(msg.id);
            pending.delete(msg.id);
            clearTimeout(timer);
            if (msg.error) reject(new CdpError(`${method}: ${msg.error.message}`));
            else resolve(msg.result ?? {});
        } else if (msg.method && listeners.has(msg.method)) {
            for (const fn of listeners.get(msg.method)) fn(msg.params ?? {});
        }
    });
    ws.on("close", () => failAll("CDP socket closed"));
    ws.on("error", (e) => failAll(`CDP socket error: ${e.message}`));

    /** Send a command; resolves with `result`, rejects on any failure. */
    const send = (method, params = {}, { timeout = timeoutMs } = {}) =>
        new Promise((resolve, reject) => {
            if (closedReason !== null || ws.readyState !== WebSocket.OPEN) {
                reject(new CdpError(`${method}: CDP socket is not open${closedReason ? ` (${closedReason})` : ""}`));
                return;
            }
            const id = ++nextId;
            const timer = setTimeout(() => {
                if (!pending.has(id)) return;
                pending.delete(id);
                reject(new CdpError(`${method}: no response within ${timeout} ms`));
            }, timeout);
            pending.set(id, { resolve, reject, timer, method });
            ws.send(JSON.stringify({ id, method, params }), (err) => {
                if (err && pending.has(id)) {
                    pending.delete(id);
                    clearTimeout(timer);
                    reject(new CdpError(`${method}: ${err.message}`));
                }
            });
        });

    /** Evaluate an expression in the page; rejects on a thrown exception. */
    const evaluate = async (expression, { timeout = timeoutMs } = {}) => {
        const r = await send(
            "Runtime.evaluate",
            { expression, awaitPromise: true, returnByValue: true, timeout },
            { timeout: timeout + 5_000 }
        );
        if (r.exceptionDetails) {
            const d = r.exceptionDetails;
            throw new CdpError(d.exception?.description ?? d.text ?? JSON.stringify(d));
        }
        return r.result?.value;
    };

    const on = (method, fn) => {
        if (!listeners.has(method)) listeners.set(method, new Set());
        listeners.get(method).add(fn);
        return () => listeners.get(method)?.delete(fn);
    };

    const close = () =>
        new Promise((resolve) => {
            if (ws.readyState === WebSocket.CLOSED) return resolve();
            ws.once("close", () => resolve());
            ws.close();
        });

    return {
        send,
        evaluate,
        on,
        close,
        get closed() {
            return closedReason !== null;
        },
    };
}

async function openSocket(url, timeoutMs) {
    const ws = new WebSocket(url, { perMessageDeflate: false, maxPayload: 1 << 28 });
    await new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
            ws.terminate();
            reject(new CdpError(`connecting to ${url}: timed out`));
        }, timeoutMs);
        ws.once("open", () => {
            clearTimeout(timer);
            resolve();
        });
        ws.once("error", (e) => {
            clearTimeout(timer);
            reject(new CdpError(`connecting to ${url}: ${e.message}`));
        });
    });
    return ws;
}

/**
 * Refuse the production instance unless explicitly allowed: these scripts
 * inject synthetic content and type into composers.
 */
export function assertDevPort(port, { allowProduction = false } = {}) {
    if (Number(port) === PRODUCTION_CDP_PORT && !allowProduction) {
        throw new CdpError(
            `port ${port} is the production AgentMux instance; point at a dev build (usually 9223) or pass --allow-production`
        );
    }
}

/** List CDP targets on a port. */
export async function listTargets(port, { host = "127.0.0.1" } = {}) {
    const res = await fetch(`http://${host}:${port}/json`);
    if (!res.ok) throw new CdpError(`GET /json on ${port}: HTTP ${res.status}`);
    return res.json();
}

/**
 * Connect to the page target whose id, title or url contains `selector`.
 * Returns `{ session, target }`.
 */
export async function connectPage(port, selector = "", { host = "127.0.0.1", timeoutMs = 10_000, ...opts } = {}) {
    const targets = await listTargets(port, { host });
    const target = targets.find(
        (t) => t.type === "page" && (t.id.includes(selector) || t.title.includes(selector) || t.url.includes(selector))
    );
    if (!target) throw new CdpError(`no page target on ${port} matching ${JSON.stringify(selector)}`);
    const ws = await openSocket(target.webSocketDebuggerUrl, timeoutMs);
    return { session: cdpSession(ws, opts), target };
}

/** Connect to the browser target (process list, SystemInfo). */
export async function connectBrowser(port, { host = "127.0.0.1", timeoutMs = 10_000, ...opts } = {}) {
    const res = await fetch(`http://${host}:${port}/json/version`);
    if (!res.ok) throw new CdpError(`GET /json/version on ${port}: HTTP ${res.status}`);
    const { webSocketDebuggerUrl } = await res.json();
    const ws = await openSocket(webSocketDebuggerUrl, timeoutMs);
    return cdpSession(ws, opts);
}
