// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The CDP session the screenshot and recording runners share (capture.mjs,
// record.mjs): finding the app's main page target, the session with its
// click/hover/type helpers, the page-dialog guard (spec §11.1), and
// measuring a selector into a clip. Node's native WebSocket; no npm
// dependencies.


export async function findMainTarget(port) {
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

export class CdpSession {
    constructor(wsUrl) {
        this.ws = new WebSocket(wsUrl);
        this.nextId = 1;
        this.pending = new Map();
        /** CDP events by method, for on()/off() (the recorder's screencast frames). */
        this.listeners = new Map();
        this.closed = false;
        this.ws.onmessage = (ev) => {
            const msg = JSON.parse(ev.data);
            if (msg.method && msg.id == null) {
                for (const fn of this.listeners.get(msg.method) ?? []) fn(msg.params);
                return;
            }
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

    /** Calls `fn(params)` for each CDP event `method`. */
    on(method, fn) {
        if (!this.listeners.has(method)) this.listeners.set(method, new Set());
        this.listeners.get(method).add(fn);
    }

    off(method, fn) {
        this.listeners.get(method)?.delete(fn);
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
        const result = await this.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
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

/** Replaces the page's blocking dialogs: confirm() answers Cancel (the safe
 *  answer to "Discard changes?" and "Delete …?" alike), alert() does nothing,
 *  prompt() answers nothing. Each is recorded in window.__screenshotDialogs. */
export const NO_DIALOGS = `(() => {
    const log = (kind, message) => { (window.__screenshotDialogs ||= []).push({ kind, message: String(message ?? "") }); };
    window.confirm = (m) => { log("confirm", m); return false; };
    window.alert = (m) => { log("alert", m); };
    window.prompt = (m) => { log("prompt", m); return null; };
})()`;

/** Resolves a CSS selector's bounding box (with optional padding) into a CDP
 *  `clip` region, from its first match. An array of selectors crops to the
 *  box around each one's first match (a menu and its open submenu, say);
 *  every one must match. */
export async function resolveClip(session, selector, padding = 0) {
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
