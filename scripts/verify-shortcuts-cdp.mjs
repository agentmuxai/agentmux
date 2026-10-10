// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the shortcut verification scripts share: the DevTools connection to a
// running AgentMux window, key sending, window cleanup, and the rows left to a
// person. Used by verify-shortcuts.mjs (L1/L2) and
// verify-shortcuts-l3-linux.mjs (L3).

/** Commands this script leaves to a person, and why. */
export const MANUAL = {
    "pane:close": "closes a pane (refused to agents: ClosePane has the undo)",
    "files:deletePermanently": "deletes for good (refused to agents)",
    "tab:close": "closes a tab",
    "doctab:close": "closes a document",
    "files:closeTab": "closes a tab",
    "open:agent": "opens an agent",
    "pane:voice": "asks for the microphone",
    "files:mention": "puts paths in an agent's message box",
    "files:openInNewTab": "opens the selected files in their apps",
    // A line break in the clipboard (files:copy puts one between paths) would
    // run each line in the shell.
    "term:paste": "pastes the clipboard into a shell",
};

export async function attach(target) {
    const ws = new WebSocket(target.webSocketDebuggerUrl);
    let seq = 0;
    const pending = new Map();
    ws.onmessage = (ev) => {
        const m = JSON.parse(ev.data);
        if (m.id && pending.has(m.id)) {
            pending.get(m.id)(m);
            pending.delete(m.id);
        }
    };
    await new Promise((r, j) => {
        ws.onopen = r;
        ws.onerror = () => j(new Error(`CDP websocket for ${target.url} failed`));
    });
    // Every call gives up after CALL_TIMEOUT_MS: a page that goes away (a
    // window closing, a dev reload) never answers, and the run would hang.
    const send = (method, params = {}) =>
        new Promise((r, j) => {
            const id = ++seq;
            const timer = setTimeout(() => {
                pending.delete(id);
                j(new Error(`${method} got no answer in ${CALL_TIMEOUT_MS / 1000} s`));
            }, CALL_TIMEOUT_MS);
            pending.set(id, (m) => {
                clearTimeout(timer);
                r(m);
            });
            ws.send(JSON.stringify({ id, method, params }));
        });
    /** Sends a call without waiting for its answer (one that may never come). */
    const fire = (method, params = {}) => ws.send(JSON.stringify({ id: ++seq, method, params }));
    const evaluate = async (expression) => {
        const m = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
        if (m.result?.exceptionDetails) throw new Error(m.result.exceptionDetails.exception?.description ?? m.result.exceptionDetails.text);
        return m.result?.result?.value;
    };
    return { ws, send, fire, evaluate };
}

export const CALL_TIMEOUT_MS = 15_000;

export async function connect(port, pane) {
    // A dev build's pages reload whenever a file in the checkout changes, and
    // have no shortcut API until the app has started again: keep trying for
    // about 30 s.
    for (let attempt = 1; ; attempt++) {
        try {
            return await connectOnce(port, pane);
        } catch (e) {
            if (attempt >= 10 || e.message.startsWith("no CDP server")) throw e;
            await sleep(3000);
        }
    }
}

/**
 * The window to test: one with the shortcut API that is shown (macOS keeps
 * hidden pool windows with zero width) and, when `pane` is given, holds it.
 */
async function connectOnce(port, pane) {
    let targets;
    try {
        targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
    } catch (e) {
        throw new Error(`no CDP server on port ${port} (${e.message}): set --port or AGENTMUX_CDP_PORT`);
    }
    const probe = `typeof window.__agentmux_shortcuts === "object" && window.innerWidth > 0${
        pane ? ` && !!document.querySelector(${JSON.stringify(`[data-blockid="${pane}"]`)})` : ""
    }`;
    let sawApi = false;
    for (const t of targets.filter((t) => t.type === "page")) {
        let cdp;
        try {
            cdp = await attach(t);
            if (await cdp.evaluate(probe)) return cdp;
            if (await cdp.evaluate(`typeof window.__agentmux_shortcuts === "object"`)) sawApi = true;
        } catch {
            // Not a page we can drive; try the next.
        }
        await close(cdp);
    }
    if (!sawApi) throw new Error("no window here has the shortcut API (window.__agentmux_shortcuts): build from a branch that has it");
    throw new Error(pane ? `no shown window holds pane ${pane}` : "no shown window has the shortcut API");
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Closes a session and waits for it, so the process doesn't exit mid-close. */
export async function close(cdp) {
    if (!cdp || cdp.ws.readyState === WebSocket.CLOSED) return;
    const closed = new Promise((r) => cdp.ws.addEventListener("close", r, { once: true }));
    cdp.ws.close();
    await Promise.race([closed, sleep(1000)]);
}

/** The extra app windows open now (window:new may claim a pre-warmed one,
 *  which then drops its `pool=1`), by DevTools target id. */
export async function appWindows(port) {
    const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
    return targets.filter((t) => t.type === "page" && /[?&]windowLabel=window-/.test(t.url) && !/[?&]pool=1/.test(t.url));
}

/** Closes the app windows a row opened (window:new), so runs don't pile them up. */
export async function closeNewWindows(port, before) {
    await sleep(300);
    for (const t of await appWindows(port)) {
        if (before.has(t.id)) continue;
        const w = await attach(t).catch(() => null);
        if (!w) continue;
        // The window closes before it can answer, so don't wait for a reply.
        w.fire("Runtime.evaluate", { expression: "window.api?.closeWindow?.()" });
        await close(w);
    }
}

export const js = (v) => (v === undefined ? "undefined" : JSON.stringify(v));

/** Sends one planned key the way the host's press_keys does. */
export async function sendKey(cdp, ev) {
    for (const type of ["rawKeyDown", "keyUp"]) {
        await cdp.send("Input.dispatchKeyEvent", { type, key: ev.key, code: ev.code, windowsVirtualKeyCode: ev.keyCode, modifiers: ev.modifiers });
    }
}

/** Escape, to close whatever a command opened (a dialog, the palette). */
export async function settle(cdp) {
    await sleep(150);
    await sendKey(cdp, { key: "Escape", code: "Escape", keyCode: 27, modifiers: 0 });
    await sleep(100);
}

export const S = "window.__agentmux_shortcuts";
