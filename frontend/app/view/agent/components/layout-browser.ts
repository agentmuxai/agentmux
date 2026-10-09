// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Measuring a page in a real headless browser, for layout tests jsdom can't do
 * (it has no layout). Used by `*.layout.test.tsx`.
 *
 * The page writes its measurements as JSON into `<pre id="result">`. The
 * browser is run with `--dump-dom` and stopped (by its own PID) as soon as that
 * element appears in its output: on macOS headless Chrome dumps the DOM but
 * doesn't always exit, which made a run that waited for it hit its timeout.
 */

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

/** A Chromium-based browser on this machine, or null (layout tests then skip).
 *  `AGENTMUX_TEST_BROWSER` forces one. */
export function findBrowser(): string | null {
    const candidates = [
        process.env.AGENTMUX_TEST_BROWSER,
        "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
        "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ];
    return candidates.find((c): c is string => !!c && existsSync(c)) ?? null;
}

const RESULT_RE = /<pre id="result">([\s\S]*?)<\/pre>/;

const unescape = (s: string) =>
    s
        .replace(/&quot;/g, '"')
        .replace(/&lt;/g, "<")
        .replace(/&gt;/g, ">")
        .replace(/&amp;/g, "&");

/**
 * Load `html` (a whole document whose script fills `<pre id="result">`) in
 * `browser` and return the parsed result.
 */
export function measureInBrowser<T>(
    browser: string,
    html: string,
    opts: { width: number; height: number; timeoutMs?: number }
): Promise<T> {
    const dir = mkdtempSync(join(tmpdir(), "agentmux-layout-"));
    const file = join(dir, "fixture.html");
    writeFileSync(file, html);
    return new Promise<T>((resolve, reject) => {
        const child = spawn(
            browser,
            [
                "--headless=new",
                "--disable-gpu",
                "--no-sandbox",
                "--no-first-run",
                "--hide-scrollbars",
                `--user-data-dir=${join(dir, "profile")}`,
                `--window-size=${opts.width},${opts.height}`,
                "--virtual-time-budget=3000",
                "--dump-dom",
                pathToFileURL(file).href,
            ],
            { stdio: ["ignore", "pipe", "ignore"] }
        );
        let out = "";
        let settled = false;
        const finish = (err: Error | null, value?: T) => {
            if (settled) return;
            settled = true;
            clearTimeout(timer);
            if (child.exitCode === null) child.kill("SIGKILL");
            try {
                rmSync(dir, { recursive: true, force: true });
            } catch {
                // the profile can still be locked for a moment on Windows
            }
            if (err) reject(err);
            else resolve(value as T);
        };
        const timer = setTimeout(
            () => finish(new Error(`no measurements after ${opts.timeoutMs ?? 60_000} ms:\n${out.slice(0, 2000)}`)),
            opts.timeoutMs ?? 60_000
        );
        child.stdout.setEncoding("utf8");
        child.stdout.on("data", (chunk: string) => {
            out += chunk;
            const m = RESULT_RE.exec(out);
            if (m) {
                try {
                    finish(null, JSON.parse(unescape(m[1])) as T);
                } catch (e) {
                    finish(e as Error);
                }
            }
        });
        child.on("error", (e) => finish(e));
        child.on("exit", () => {
            const m = RESULT_RE.exec(out);
            if (!m) finish(new Error("the page produced no measurements:\n" + out.slice(0, 2000)));
        });
    });
}
