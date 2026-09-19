#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Reusable, cropped screenshot capture for the AgentMux UI manual.
// See docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md for the
// full design — this is the runner; docs/specs' companion `shots.mjs`
// (loaded from this same directory) is the declarative manifest of what
// to capture. Edit shots.mjs to add/change shots; this file shouldn't
// need to change for the common case.
//
// Usage:
//   node scripts/ui-screenshots/capture.mjs [--port N] [--out DIR] [--only id1,id2]
//
// Talks directly to the target AgentMux instance's CDP remote-debugging
// port — NOT the agentmux-mcp UIScreenshot/UIClick/UIQuery tools, which are
// scoped to the calling agent's own pane and can't reach a separate
// instance's window. See the spec's §2 for why. Node's native WebSocket
// (no external dependency) is used throughout.

import { writeFileSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));

function parseArgs(argv) {
    const args = { port: Number(process.env.AGENTMUX_CDP_PORT) || 9222, out: null, only: null };
    for (let i = 0; i < argv.length; i++) {
        const a = argv[i];
        if (a === "--port") args.port = Number(argv[++i]);
        else if (a === "--out") args.out = argv[++i];
        else if (a === "--only") args.only = argv[++i].split(",").map((s) => s.trim());
    }
    if (!args.out) {
        const stamp = new Date().toISOString().replace(/[:.]/g, "-");
        args.out = join(__dirname, "..", "..", "docs", "manual", "screenshots", stamp);
    }
    return args;
}

async function findMainTarget(port) {
    const res = await fetch(`http://127.0.0.1:${port}/json`);
    const targets = await res.json();
    const pages = targets.filter((t) => t.type === "page");
    // The main window's URL has no windowLabel= query param — pool/
    // floating-pool pre-warmed windows always do (?windowLabel=...&pool=1
    // or &pane-pool=1), confirmed live across every instance inspected
    // this session.
    const main = pages.find((t) => !t.url.includes("windowLabel="));
    if (!main) {
        throw new Error(
            `No main-window CDP target found on port ${port}. Targets seen:\n` +
                pages.map((p) => `  ${p.title} — ${p.url}`).join("\n")
        );
    }
    return main;
}

class CdpSession {
    constructor(wsUrl) {
        this.ws = new WebSocket(wsUrl);
        this.nextId = 1;
        this.pending = new Map();
        this.closed = false;
        this.ws.onmessage = (ev) => {
            const msg = JSON.parse(ev.data);
            if (msg.id != null && this.pending.has(msg.id)) {
                const { resolve, reject, timer } = this.pending.get(msg.id);
                clearTimeout(timer);
                this.pending.delete(msg.id);
                if (msg.error) reject(new Error(msg.error.message));
                else resolve(msg.result);
            }
        };
        // Lasting handlers (unlike connect()'s one-shot error listener below):
        // if the socket drops or errors mid-run — e.g. a prep click navigates
        // or reloads the target window — every still-pending send() must be
        // rejected immediately instead of hanging forever with no response.
        this.ws.onclose = () => this._failAll("CDP connection closed unexpectedly");
        this.ws.onerror = (e) => this._failAll(`CDP connection error: ${e.message ?? e}`);
    }

    _failAll(reason) {
        if (this.closed) return;
        this.closed = true;
        const err = new Error(reason);
        for (const { reject, timer } of this.pending.values()) {
            clearTimeout(timer);
            reject(err);
        }
        this.pending.clear();
    }

    static async connect(wsUrl) {
        const session = new CdpSession(wsUrl);
        await new Promise((resolve, reject) => {
            session.ws.onopen = () => resolve();
            session.ws.addEventListener("error", (e) => reject(new Error(`CDP connect failed: ${e.message ?? e}`)), { once: true });
        });
        return session;
    }

    send(method, params = {}, timeoutMs = 10000) {
        if (this.closed) return Promise.reject(new Error(`CDP session closed, cannot send ${method}`));
        const id = this.nextId++;
        return new Promise((resolve, reject) => {
            const timer = setTimeout(() => {
                this.pending.delete(id);
                reject(new Error(`CDP command "${method}" timed out after ${timeoutMs}ms (id=${id})`));
            }, timeoutMs);
            this.pending.set(id, { resolve, reject, timer });
            this.ws.send(JSON.stringify({ id, method, params }));
        });
    }

    /** Evaluates `expression` in the page and returns its value (must be JSON-serializable). */
    async evaluate(expression) {
        const result = await this.send("Runtime.evaluate", { expression, returnByValue: true });
        if (result.exceptionDetails) {
            throw new Error(`evaluate() threw: ${result.exceptionDetails.text}`);
        }
        return result.result.value;
    }

    /** Clicks the center of the first element matching `selector`. Throws if not found. */
    async clickSelector(selector) {
        const rect = await this.evaluate(
            `(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return null; const r = el.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`
        );
        if (!rect) throw new Error(`clickSelector: no element matches ${selector}`);
        await this.send("Input.dispatchMouseEvent", { type: "mousePressed", x: rect.x, y: rect.y, button: "left", clickCount: 1 });
        await this.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: rect.x, y: rect.y, button: "left", clickCount: 1 });
    }

    /** Clicks the first element within `containerSelector` (default: whole
     *  document) whose trimmed text content equals or contains `text`.
     *  Prefers an exact match; falls back to substring. More robust than a
     *  guessed CSS class for widget-bar buttons/menu items, whose class
     *  names are often Tailwind utility soup with no semantic hook. */
    async clickText(containerSelector, text) {
        const rect = await this.evaluate(`(() => {
            const root = ${containerSelector ? `document.querySelector(${JSON.stringify(containerSelector)})` : "document"};
            if (!root) return null;
            const all = [...root.querySelectorAll("*")].filter((el) => el.children.length === 0);
            const norm = (s) => s.trim();
            let el = all.find((e) => norm(e.textContent) === ${JSON.stringify(text)});
            if (!el) el = all.find((e) => norm(e.textContent).includes(${JSON.stringify(text)}));
            if (!el) return null;
            const clickable = el.closest("button, a, [role=button], [role=menuitem]") ?? el;
            const r = clickable.getBoundingClientRect();
            return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
        })()`);
        if (!rect) throw new Error(`clickText: no element with text "${text}" under ${containerSelector ?? "document"}`);
        await this.send("Input.dispatchMouseEvent", { type: "mousePressed", x: rect.x, y: rect.y, button: "left", clickCount: 1 });
        await this.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: rect.x, y: rect.y, button: "left", clickCount: 1 });
    }

    async wait(ms) {
        await new Promise((r) => setTimeout(r, ms));
    }

    close() {
        // Mark closed first so the onclose handler's _failAll() no-ops —
        // this is an intentional shutdown, not a dropped-connection failure.
        this.closed = true;
        this.ws.close();
    }
}

/** Reads width/height straight out of a PNG's IHDR chunk (bytes 16-23,
 *  big-endian uint32 each) — the actual captured pixel dimensions,
 *  independent of any CSS-pixel/device-scale-factor math, and available
 *  whether or not the shot used a `clip` (a `selector`). */
function pngDimensions(buffer) {
    return { width: buffer.readUInt32BE(16), height: buffer.readUInt32BE(20) };
}

/** Resolves a CSS selector's bounding box (with optional padding) into a CDP `clip` region. */
async function resolveClip(session, selector, padding = 0) {
    const rect = await session.evaluate(
        `(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return null; const r = el.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height }; })()`
    );
    if (!rect) return null;
    return {
        x: Math.max(0, rect.x - padding),
        y: Math.max(0, rect.y - padding),
        width: rect.width + padding * 2,
        height: rect.height + padding * 2,
        scale: 1,
    };
}

async function main() {
    const args = parseArgs(process.argv.slice(2));
    const { shots } = await import("./shots.mjs");
    const selected = args.only ? shots.filter((s) => args.only.includes(s.id)) : shots;
    if (selected.length === 0) {
        console.error("No shots selected — check --only against shots.mjs's ids.");
        process.exit(1);
    }

    mkdirSync(args.out, { recursive: true });
    console.log(`Connecting to CDP on port ${args.port}...`);
    const target = await findMainTarget(args.port);
    console.log(`Main target: ${target.title} — ${target.url}`);
    const session = await CdpSession.connect(target.webSocketDebuggerUrl);
    await session.send("Runtime.enable");

    const manifest = [];
    const failures = [];

    for (let i = 0; i < selected.length; i++) {
        const shot = selected[i];
        const n = String(i + 1).padStart(2, "0");
        process.stdout.write(`[${n}/${selected.length}] ${shot.id} ... `);
        try {
            if (shot.prep) await shot.prep(session);
            await session.wait(shot.settleMs ?? 400);

            let clip;
            if (shot.selector) {
                clip = await resolveClip(session, shot.selector, shot.padding ?? 0);
                if (!clip) throw new Error(`selector not found: ${shot.selector}`);
            }

            const { data } = await session.send("Page.captureScreenshot", {
                format: "png",
                ...(clip ? { clip } : {}),
            });
            const filename = `${n}-${shot.id}.png`;
            const pngBuffer = Buffer.from(data, "base64");
            writeFileSync(join(args.out, filename), pngBuffer);
            const { width, height } = pngDimensions(pngBuffer);

            manifest.push({
                id: shot.id,
                title: shot.title,
                description: shot.description,
                file: filename,
                capturedAt: new Date().toISOString(),
                width,
                height,
            });
            console.log("ok");
        } catch (err) {
            console.log(`FAILED — ${err.message}`);
            failures.push({ id: shot.id, error: err.message });
        }
    }

    writeFileSync(join(args.out, "manifest.json"), JSON.stringify({ capturedAt: new Date().toISOString(), shots: manifest, failures }, null, 2));
    session.close();

    console.log(`\n${manifest.length}/${selected.length} captured to ${args.out}`);
    if (failures.length) {
        console.log(`${failures.length} FAILED:`);
        for (const f of failures) console.log(`  - ${f.id}: ${f.error}`);
        process.exitCode = 1;
    }
}

main().catch((err) => {
    console.error(err);
    process.exit(1);
});
