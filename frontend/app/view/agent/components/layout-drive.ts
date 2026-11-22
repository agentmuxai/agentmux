// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Driving a page in a real headless browser over the DevTools protocol, for tests that need the engine's own
 * input handling (a selection drag, which jsdom cannot do). The sibling of `layout-browser.ts`, which only
 * measures. Used by `*.layout.test.ts` that skip with no usable browser.
 *
 * The browser is started with its own profile and a debugging port it picks, and is stopped by its own PID.
 */
import { spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { pathToFileURL } from "node:url";

export interface DrivenPage {
    /** Evaluate an expression in the page and return its value. */
    eval<T>(expression: string): Promise<T>;
    /** Dispatch a primary-button mouse event at page coordinates. */
    mouse(type: "mouseMoved" | "mousePressed" | "mouseReleased", x: number, y: number, opts?: { clickCount?: number; modifiers?: number }): Promise<void>;
    /** A key press (down and up) with CDP modifier bits (2 Ctrl, 8 Shift) and, for a selection key, the editing commands it runs. */
    key(key: string, code: string, vk: number, modifiers?: number, commands?: string[]): Promise<void>;
}

/** A page with `html` loaded in `browser`; `run` drives it, then the browser is stopped. */
export async function driveInBrowser<T>(
    browser: string,
    html: string,
    size: { width: number; height: number },
    run: (page: DrivenPage) => Promise<T>
): Promise<T> {
    const dir = mkdtempSync(join(tmpdir(), "agentmux-drive-"));
    const file = join(dir, "fixture.html");
    writeFileSync(file, html);
    const child = spawn(
        browser,
        [
            ...(basename(browser).startsWith("chrome-headless-shell") ? [] : ["--headless=new"]),
            "--disable-gpu",
            "--no-sandbox",
            "--no-first-run",
            "--hide-scrollbars",
            "--remote-debugging-port=0",
            `--user-data-dir=${join(dir, "profile")}`,
            `--window-size=${size.width},${size.height}`,
            pathToFileURL(file).href,
        ],
        { stdio: ["ignore", "ignore", "pipe"] }
    );
    try {
        const port = await debuggingPort(child);
        const ws = await openPageSocket(port);
        try {
            const page = pageOver(ws);
            for (let i = 0; i < 100 && (await page.eval<string>("document.readyState")) !== "complete"; i++) await new Promise((r) => setTimeout(r, 100));
            return await run(page);
        } finally {
            ws.close();
        }
    } finally {
        if (child.exitCode === null) child.kill("SIGKILL");
        try {
            rmSync(dir, { recursive: true, force: true });
        } catch {
            // the profile can still be locked for a moment on Windows
        }
    }
}

function debuggingPort(child: ChildProcess): Promise<number> {
    return new Promise((resolve, reject) => {
        let seen = "";
        const timer = setTimeout(() => reject(new Error(`no debugging port:\n${seen.slice(0, 1000)}`)), 30_000);
        child.stderr!.setEncoding("utf8");
        child.stderr!.on("data", (chunk: string) => {
            seen += chunk;
            const m = /DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)\//.exec(seen);
            if (m) {
                clearTimeout(timer);
                resolve(Number(m[1]));
            }
        });
        child.on("error", reject);
    });
}

async function openPageSocket(port: number): Promise<WebSocket> {
    for (let i = 0; i < 60; i++) {
        const list = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()) as Array<{ type: string; url: string; webSocketDebuggerUrl: string }>;
        const page = list.find((t) => t.type === "page" && t.url.startsWith("file:"));
        if (page) {
            return await new Promise<WebSocket>((resolve, reject) => {
                const ws = new WebSocket(page.webSocketDebuggerUrl);
                ws.addEventListener("open", () => resolve(ws));
                ws.addEventListener("error", () => reject(new Error("the page's debugging socket failed")));
            });
        }
        await new Promise((r) => setTimeout(r, 250));
    }
    throw new Error("the fixture page never appeared");
}

function pageOver(ws: WebSocket): DrivenPage {
    let id = 0;
    const call = (method: string, params: unknown): Promise<any> =>
        new Promise((resolve, reject) => {
            const mine = ++id;
            const onMessage = (ev: MessageEvent) => {
                const msg = JSON.parse(String(ev.data));
                if (msg.id !== mine) return;
                ws.removeEventListener("message", onMessage);
                if (msg.error) reject(new Error(JSON.stringify(msg.error)));
                else resolve(msg.result);
            };
            ws.addEventListener("message", onMessage);
            ws.send(JSON.stringify({ id: mine, method, params }));
        });
    return {
        async eval<T>(expression: string): Promise<T> {
            const r = await call("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
            if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
            return r.result.value as T;
        },
        async mouse(type, x, y, opts) {
            await call("Input.dispatchMouseEvent", {
                type,
                x,
                y,
                button: "left",
                buttons: type === "mouseReleased" ? 0 : 1,
                clickCount: opts?.clickCount ?? 1,
                modifiers: opts?.modifiers ?? 0,
            });
        },
        async key(key, code, vk, modifiers = 0, commands) {
            const ev = { key, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers, ...(commands ? { commands } : {}) };
            await call("Input.dispatchKeyEvent", { type: "rawKeyDown", ...ev });
            await call("Input.dispatchKeyEvent", { type: "keyUp", ...ev });
        },
    };
}
