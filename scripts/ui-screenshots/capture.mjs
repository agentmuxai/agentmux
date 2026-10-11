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
//       [--suite manual|widgets|chrome] [--sizes small,medium,large|all]
//       [--redact from=to]... [--no-redact]
//
// --suite picks the manifest: `manual` (shots.mjs, the default), `widgets`
// (widget-shots.mjs, every widget in every size) or `chrome` (chrome-shots.mjs:
// menus, the status bar and Settings, spec §9). --sizes limits which of
// sizes.mjs's sizes a shot with `sizes: true` is captured in (default: all);
// see the spec's §8.
//
// Before each capture the capturing machine's user name and hostname are
// replaced in the page with neutral ones (redact.mjs, spec §9); --redact adds
// a pair, --no-redact turns it off. After each shot's `prep`, its `verify`
// (verify.mjs) must hold or the shot fails.
//
// Talks directly to the target AgentMux instance's CDP remote-debugging
// port — NOT the agentmux-mcp UIScreenshot/UIClick/UIQuery tools, which are
// scoped to the calling agent's own pane and can't reach a separate
// instance's window. See the spec's §2 for why. Node's native WebSocket
// (no external dependency) is used throughout.
//
// A RELEASE instance only runs that port when launched with
// AGENTMUX_CDP_PORT set (#3681) — e.g. `AGENTMUX_CDP_PORT=9222` for both the
// app and this script. Dev builds run it by default.

import { writeFileSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { SIZES, parseSizes, shotFilename } from "./sizes.mjs";
import { machinePairs, parseRedactArg, redactExpression } from "./redact.mjs";
import { checkVerify } from "./verify.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));

/** Suite name → manifest module (relative to this directory). */
const SUITES = { manual: "./shots.mjs", widgets: "./widget-shots.mjs", chrome: "./chrome-shots.mjs" };

function parseArgs(argv) {
    const args = {
        port: Number(process.env.AGENTMUX_CDP_PORT) || 9222,
        out: null,
        only: null,
        suite: "manual",
        sizes: null,
        redact: true,
        redactExtra: [],
    };
    for (let i = 0; i < argv.length; i++) {
        const a = argv[i];
        if (a === "--port") args.port = Number(argv[++i]);
        else if (a === "--out") args.out = argv[++i];
        else if (a === "--only") args.only = argv[++i].split(",").map((s) => s.trim());
        else if (a === "--suite") args.suite = argv[++i];
        else if (a === "--sizes") args.sizes = argv[++i];
        else if (a === "--redact") args.redactExtra.push(parseRedactArg(argv[++i]));
        else if (a === "--no-redact") args.redact = false;
    }
    if (!SUITES[args.suite]) throw new Error(`unknown suite "${args.suite}" (known: ${Object.keys(SUITES).join(", ")})`);
    args.sizes = parseSizes(args.sizes);
    if (!args.out) {
        const stamp = new Date().toISOString().replace(/[:.]/g, "-");
        args.out = join(__dirname, "..", "..", "docs", "manual", "screenshots", `${args.suite}-${stamp}`);
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
    // A Browser pane's web page is a page target too, on its own (external)
    // URL, so the main window is also the one served by the app itself.
    const isApp = (t) => /^https?:\/\/(127\.0\.0\.1|localhost)[:/]/.test(t.url);
    const main = pages.find((t) => isApp(t) && !t.url.includes("windowLabel="));
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
    async clickText(containerSelector, text, itemSelector = "*") {
        const rect = await this.evaluate(`(() => {
            const root = ${containerSelector ? `document.querySelector(${JSON.stringify(containerSelector)})` : "document"};
            if (!root) return null;
            const all = [...root.querySelectorAll(${JSON.stringify(itemSelector)})].filter((el) => el.children.length === 0 && el.offsetParent);
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

    /** Clicks at page coordinates `{x, y}`. */
    async clickAt({ x, y }) {
        await this.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 });
        await this.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 });
    }

    /** Right-clicks at page coordinates `{x, y}` (opens the page's context menu). */
    async rightClickAt({ x, y }) {
        await this.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
        await this.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "right", clickCount: 1 });
        await this.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "right", clickCount: 1 });
    }

    /** Moves the pointer to page coordinates `{x, y}` (opens a submenu on hover). */
    async hoverAt({ x, y }) {
        await this.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
    }

    /** Centre of the first element matching `selector`, or null. */
    async centerOf(selector) {
        return this.evaluate(
            `(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return null; const r = el.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`
        );
    }

    /** Centre of the first leaf element under `containerSelector` whose text
     *  is exactly `text` (else contains it), as clickText finds it, or null. */
    async centerOfText(containerSelector, text) {
        return this.evaluate(`(() => {
            const root = ${containerSelector ? `document.querySelector(${JSON.stringify(containerSelector)})` : "document"};
            if (!root) return null;
            const all = [...root.querySelectorAll("*")].filter((el) => el.children.length === 0 && el.offsetParent);
            let el = all.find((e) => e.textContent.trim() === ${JSON.stringify(text)});
            if (!el) el = all.find((e) => e.textContent.trim().includes(${JSON.stringify(text)}));
            if (!el) return null;
            const target = el.closest("button, a, [role=button], [role=menuitem], .menu-item") ?? el;
            const r = target.getBoundingClientRect();
            return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
        })()`);
    }

    /** Right-clicks the centre of the first element matching `selector`. */
    async rightClickSelector(selector) {
        const pt = await this.centerOf(selector);
        if (!pt) throw new Error(`rightClickSelector: no element matches ${selector}`);
        await this.rightClickAt(pt);
    }

    /** Hovers the element under `containerSelector` with text `text`. */
    async hoverText(containerSelector, text) {
        const pt = await this.centerOfText(containerSelector, text);
        if (!pt) throw new Error(`hoverText: no element with text "${text}" under ${containerSelector ?? "document"}`);
        await this.hoverAt(pt);
    }

    /** Moves the pointer off the page, so nothing captured afterwards shows a
     *  hover state or a tooltip left by the last click. */
    async parkMouse() {
        await this.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: -1, y: -1 });
        await this.wait(250);
    }

    /** Lays the page out at `width`×`height` CSS pixels (and `scale` device
     *  pixels per CSS pixel) without resizing the real window. */
    async setViewport({ width, height, scale = 1 }) {
        await this.send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: scale, mobile: false });
        await this.wait(300);
    }

    async clearViewport() {
        await this.send("Emulation.clearDeviceMetricsOverride");
    }

    /** Presses one key (a single character, or a name such as "Enter",
     *  "Escape"), with optional ctrl/shift/alt. */
    async pressKey(key, { ctrl = false, shift = false, alt = false } = {}) {
        const NAMED = { Enter: 13, Escape: 27, Tab: 9, Backspace: 8, ArrowDown: 40, ArrowUp: 38 };
        const vk = NAMED[key] ?? key.toUpperCase().charCodeAt(0);
        const modifiers = (alt ? 1 : 0) | (ctrl ? 2 : 0) | (shift ? 8 : 0);
        const code = key.length === 1 ? `Key${key.toUpperCase()}` : key;
        const event = { key, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers };
        await this.send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...event });
        await this.send("Input.dispatchKeyEvent", { type: "keyUp", ...event });
    }

    /** Types `text` into the focused element. */
    async typeText(text) {
        await this.send("Input.insertText", { text });
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

/** Resolves a CSS selector's bounding box (with optional padding) into a CDP
 *  `clip` region, from its first match. An array of selectors crops to the
 *  box around each one's first match (a menu and its open submenu, say);
 *  every one must match. */
async function resolveClip(session, selector, padding = 0) {
    const selectors = Array.isArray(selector) ? selector : [selector];
    const rect = await session.evaluate(`(() => {
        let box = null;
        for (const sel of ${JSON.stringify(selectors)}) {
            const el = document.querySelector(sel);
            if (!el) return null;
            const r = el.getBoundingClientRect();
            box = box
                ? { l: Math.min(box.l, r.left), t: Math.min(box.t, r.top), r: Math.max(box.r, r.right), b: Math.max(box.b, r.bottom) }
                : { l: r.left, t: r.top, r: r.right, b: r.bottom };
        }
        return box && { x: box.l, y: box.t, width: box.r - box.l, height: box.b - box.t };
    })()`);
    if (!rect) return null;
    return {
        x: Math.max(0, rect.x - padding),
        y: Math.max(0, rect.y - padding),
        width: rect.width + padding * 2,
        height: rect.height + padding * 2,
        scale: 1,
    };
}

/** Captures one PNG: redacts the machine's names, measures the shot's
 *  selector (a string, an array of them, or a function returning either, for
 *  a pane found during `prep`) and crops to it. `redaction`, when set, is
 *  `{ pairs, replacements }`; the count of replacements made is added to it. */
async function captureOne(session, shot, redaction) {
    // A menu that opens on hover closes when the pointer leaves it; such a
    // shot sets `keepPointer` and leaves the pointer where its prep put it.
    if (!shot.keepPointer) await session.parkMouse();
    if (redaction && redaction.pairs.length > 0) {
        // Before measuring: a shorter name changes the text's width.
        redaction.replacements += await session.evaluate(redactExpression(redaction.pairs));
        await session.evaluate("new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))");
    }
    let clip;
    const selector = typeof shot.selector === "function" ? shot.selector() : shot.selector;
    if (selector) {
        clip = await resolveClip(session, selector, shot.padding ?? 0);
        if (!clip) throw new Error(`selector not found: ${selector}`);
        if (process.env.SHOTS_DEBUG) console.log(`\n  clip ${JSON.stringify(clip)} for ${selector}`);
    }
    // A capture waits for the next compositor frame, which can be slow to come
    // when the window is behind others; give it longer, and one retry.
    const params = { format: "png", ...(clip ? { clip } : {}) };
    let lastErr;
    for (let attempt = 0; attempt < 2; attempt++) {
        try {
            const { data } = await session.send("Page.captureScreenshot", params, 20000);
            return Buffer.from(data, "base64");
        } catch (err) {
            lastErr = err;
            await session.send("Page.bringToFront").catch(() => {});
            await session.wait(500);
        }
    }
    throw lastErr;
}

async function main() {
    const args = parseArgs(process.argv.slice(2));
    const { shots } = await import(SUITES[args.suite]);
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
    await session.send("Page.bringToFront").catch(() => {});

    // The names themselves never reach the manifest or the console: only how
    // many pairs there were and how many replacements were made.
    const redaction = args.redact ? { pairs: machinePairs(undefined, args.redactExtra), replacements: 0 } : null;
    console.log(redaction ? `Redacting ${redaction.pairs.length} name(s) before each capture.` : "Redaction is off (--no-redact).");

    const manifest = [];
    const failures = [];

    for (let i = 0; i < selected.length; i++) {
        const shot = selected[i];
        const n = String(i + 1).padStart(2, "0");
        process.stdout.write(`[${n}/${selected.length}] ${shot.id} ... `);
        const variants = shot.sizes ? args.sizes : [null];
        // A shot may allow `retries` (UI timing makes the odd attempt fail);
        // each attempt is cleaned up before the next, and only the images of
        // the attempt that succeeded go into the manifest.
        const attempts = (shot.retries ?? 0) + 1;
        let entries = [];
        let lastErr = null;
        let stop = false;
        for (let attempt = 1; attempt <= attempts; attempt++) {
            entries = [];
            lastErr = null;
            try {
                if (shot.prep) await shot.prep(session);
                await session.wait(shot.settleMs ?? 400);
                await checkVerify(session, shot);

                // One capture, or one per size: the viewport is set to the size
                // and the selector re-measured, so each crop fits that layout.
                for (const size of variants) {
                    if (size) {
                        await session.setViewport(SIZES[size]);
                        await session.wait(shot.sizeSettleMs ?? 600);
                    }
                    const pngBuffer = await captureOne(session, shot, redaction);
                    const filename = shotFilename(n, shot.id, size);
                    writeFileSync(join(args.out, filename), pngBuffer);
                    const { width, height } = pngDimensions(pngBuffer);
                    entries.push({
                        id: shot.id,
                        title: shot.title,
                        description: shot.description,
                        ...(size ? { size, viewport: SIZES[size] } : {}),
                        // Whether a human must check the image for this machine's
                        // data before it's published (spec §7/§8): true, false or "review".
                        ...(shot.containsWorkspaceData !== undefined ? { containsWorkspaceData: shot.containsWorkspaceData } : {}),
                        file: filename,
                        capturedAt: new Date().toISOString(),
                        width,
                        height,
                    });
                }
            } catch (err) {
                lastErr = err;
            }
            // SHOTS_DEBUG=keep leaves the shot's state in place to inspect.
            if (process.env.SHOTS_DEBUG === "keep") break;
            // Undo the shot's setup (a tab it opened, a viewport it set) even
            // when it failed, so the next attempt or shot starts from the same
            // state.
            try {
                if (shot.cleanup) await shot.cleanup(session);
                else if (shot.sizes) await session.clearViewport();
            } catch (err) {
                // State the shot left behind (a tab that didn't close) can
                // change every later shot, so a failed cleanup fails the shot
                // and ends the run (Codex on #4471).
                lastErr = new Error(`cleanup: ${err.message}`);
                stop = true;
                break;
            }
            if (!lastErr) break;
            if (attempt < attempts) process.stdout.write(`retrying (${lastErr.message}) ... `);
        }
        if (lastErr) {
            console.log(`FAILED — ${lastErr.message}${stop ? "; stopping the run" : ""}`);
            failures.push({ id: shot.id, error: lastErr.message });
        } else {
            manifest.push(...entries);
            console.log(shot.sizes ? `ok (${variants.join(", ")})` : "ok");
        }
        if (stop) break;
    }

    writeFileSync(
        join(args.out, "manifest.json"),
        JSON.stringify(
            {
                capturedAt: new Date().toISOString(),
                suite: args.suite,
                app: target.title,
                redaction: redaction ? { names: redaction.pairs.length, replacements: redaction.replacements } : "off",
                shots: manifest,
                failures,
            },
            null,
            2
        )
    );
    session.close();

    // Shots with at least one image (a failure can come from cleanup after a
    // capture, and a stopped run leaves later shots unrun).
    const done = new Set(manifest.map((m) => m.id)).size;
    console.log(`\n${done}/${selected.length} shots captured (${manifest.length} images) to ${args.out}`);
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
