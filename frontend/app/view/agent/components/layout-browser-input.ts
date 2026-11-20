// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Driving a page in a real headless browser with real input, for tests that need the browser's own hit testing and
 * scrolling (a wheel notch, a pointer move) and not only its layout. `layout-browser.ts` dumps a page's DOM once;
 * this keeps the page open and sends it input through the DevTools protocol (`Input.dispatchMouseEvent`), which goes
 * through the same pipeline as a mouse: hit test, wheel scrolling, hover.
 *
 * The browser is stopped by its own PID when the callback returns or throws. It listens on a port the browser picks,
 * on the loopback address only.
 */

import { spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { pathToFileURL } from "node:url";

export interface InputPage {
    /** Evaluate an expression in the page and return its JSON value (a promise is awaited). */
    evaluate<T>(expression: string): Promise<T>;
    /** One wheel notch of `deltaY` px with the pointer at (x, y), in CSS px of the window. */
    wheel(x: number, y: number, deltaY: number): Promise<void>;
    /**
     * A wheel notch that is sent again, after a short wait, until `moved()` says it took effect or `tries` notches have
     * been sent; resolves with how many were sent. A notch sent before the browser has built its scroll tree is dropped
     * (a cold start on a busy machine, seen on the nightly's runner), and a dropped notch leaves nothing to add up.
     * A notch the page did receive (it counts its wheel events) is waited for about 3 s and not sent again, so a slow
     * runner never adds a second notch to the first.
     */
    wheelUntil(x: number, y: number, deltaY: number, moved: () => Promise<boolean>, tries?: number): Promise<number>;
    /** Move the pointer to (x, y). */
    move(x: number, y: number): Promise<void>;
}

type Send = (method: string, params?: object) => Promise<unknown>;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Start the browser on `file` with a DevTools port of its choosing, and resolve with that port. */
function startBrowser(
    browser: string,
    file: string,
    dir: string,
    size: { width: number; height: number },
    timeoutMs: number
): { child: ChildProcess; port: Promise<number> } {
    const child = spawn(
        browser,
        [
            ...(basename(browser).startsWith("chrome-headless-shell") ? [] : ["--headless=new"]),
            "--disable-gpu",
            "--no-sandbox",
            "--no-first-run",
            "--disable-smooth-scrolling",
            "--remote-debugging-port=0",
            `--user-data-dir=${join(dir, "profile")}`,
            `--window-size=${size.width},${size.height}`,
            pathToFileURL(file).href,
        ],
        { stdio: ["ignore", "ignore", "pipe"] }
    );
    const port = new Promise<number>((resolve, reject) => {
        let err = "";
        const timer = setTimeout(() => reject(new Error(`the browser did not start listening:\n${err}`)), timeoutMs);
        child.stderr!.setEncoding("utf8");
        child.stderr!.on("data", (chunk: string) => {
            err += chunk;
            const m = /DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)\//.exec(err);
            if (m) {
                clearTimeout(timer);
                resolve(Number(m[1]));
            }
        });
        child.on("error", reject);
        child.on("close", () => reject(new Error(`the browser exited:\n${err}`)));
    });
    return { child, port };
}

/** Connect to the browser's page and return a `send` for protocol commands, and the socket to close. */
async function connectPage(port: number): Promise<{ send: Send; socket: WebSocket }> {
    let target: { type: string; webSocketDebuggerUrl: string } | undefined;
    for (let i = 0; i < 100 && !target; i++) {
        const list = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()) as { type: string; webSocketDebuggerUrl: string }[];
        target = list.find((t) => t?.type === "page");
        if (!target) await sleep(50);
    }
    if (!target) throw new Error("the browser opened no page");
    const socket = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise<void>((resolve, reject) => {
        socket.addEventListener("open", () => resolve(), { once: true });
        socket.addEventListener("error", () => reject(new Error("could not connect to the page")), { once: true });
    });
    let nextId = 0;
    const pending = new Map<number, (m: { result?: unknown; error?: { message: string } }) => void>();
    socket.addEventListener("message", (e) => {
        const m = JSON.parse(String(e.data));
        const done = m.id != null ? pending.get(m.id) : undefined;
        if (done) {
            pending.delete(m.id);
            done(m);
        }
    });
    const send: Send = (method, params = {}) =>
        new Promise((resolve, reject) => {
            const id = ++nextId;
            pending.set(id, (m) => (m.error ? reject(new Error(`${method}: ${m.error.message}`)) : resolve(m.result)));
            socket.send(JSON.stringify({ id, method, params }));
        });
    return { send, socket };
}

function inputPage(send: Send): InputPage {
    const page: InputPage = {
        async evaluate<V>(expression: string) {
            const r = (await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true })) as {
                result: { value: V };
                exceptionDetails?: unknown;
            };
            if (r.exceptionDetails) throw new Error(`in the page: ${JSON.stringify(r.exceptionDetails).slice(0, 400)}`);
            return r.result.value;
        },
        async wheel(x, y, deltaY) {
            await send("Input.dispatchMouseEvent", { type: "mouseWheel", x, y, deltaX: 0, deltaY });
        },
        async wheelUntil(x, y, deltaY, moved, tries = 8) {
            // The page counts the wheel events it receives, so a notch that is only slow (a loaded runner) is told
            // from one that was dropped: the slow one is waited for, never sent again, because a second notch on
            // top of it adds up to a scroll of twice the amount.
            await page.evaluate<boolean>(
                `(() => { if (window.__wheelSeen === undefined) { window.__wheelSeen = 0;
                  addEventListener('wheel', () => { window.__wheelSeen++; }, { capture: true, passive: true }); } return true; })()`
            );
            const seen = () => page.evaluate<number>("window.__wheelSeen");
            const waitFor = async (polls: number) => {
                for (let i = 0; i < polls; i++) {
                    await sleep(50);
                    if (await moved()) return true;
                }
                return false;
            };
            for (let sent = 1; ; sent++) {
                const before = await seen();
                await page.wheel(x, y, deltaY);
                // Give a notch that was taken time to show; one that was dropped never does.
                if (await waitFor(8)) return sent;
                // The page got it but has not scrolled yet: wait longer (about 3 s) before sending another.
                if ((await seen()) > before && (await waitFor(60))) return sent;
                if (sent >= tries) return sent;
            }
        },
        async move(x, y) {
            await send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
        },
    };
    return page;
}

/** Open `html` in `browser`, run `fn` with a page that takes real input, and stop the browser. */
export async function withInputPage<T>(
    browser: string,
    html: string,
    opts: { width: number; height: number; timeoutMs?: number },
    fn: (page: InputPage) => Promise<T>
): Promise<T> {
    const dir = mkdtempSync(join(tmpdir(), "agentmux-input-"));
    const file = join(dir, "fixture.html");
    writeFileSync(file, html);
    const { child, port } = startBrowser(browser, file, dir, opts, opts.timeoutMs ?? 60_000);
    let socket: WebSocket | undefined;
    try {
        const conn = await connectPage(await port);
        socket = conn.socket;
        const page = inputPage(conn.send);
        // The page has loaded when it is the fixture (the target starts on about:blank) and is complete.
        for (let i = 0; i < 200; i++) {
            if (await page.evaluate<boolean>("location.protocol === 'file:' && document.readyState === 'complete'")) break;
            await sleep(25);
        }
        // Loaded is not yet drawn: input sent before the page's first frames reaches a page whose scroller the
        // compositor does not know yet, and a wheel notch then does nothing (seen once on CI, a cold browser
        // start). Two animation frames in the page mean it has been drawn.
        await page.evaluate<boolean>("new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r(true))))");
        return await fn(page);
    } finally {
        socket?.close();
        if (child.exitCode === null) child.kill("SIGKILL");
        try {
            rmSync(dir, { recursive: true, force: true });
        } catch {
            // the profile can still be locked for a moment on Windows
        }
    }
}
