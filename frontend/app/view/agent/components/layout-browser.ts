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
import { existsSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { basename, join } from "node:path";
import { pathToFileURL } from "node:url";

/** What browser lookup can see; the real machine by default, a fake in tests. */
export interface BrowserLookup {
    env: Record<string, string | undefined>;
    platform: NodeJS.Platform;
    arch: string;
    home: string;
    exists: (path: string) => boolean;
    /** Entries of a directory, or [] when it doesn't exist. */
    list: (dir: string) => string[];
}

const realLookup = (): BrowserLookup => ({
    env: process.env,
    platform: process.platform,
    arch: process.arch,
    home: homedir(),
    exists: existsSync,
    list: (dir) => {
        try {
            return readdirSync(dir);
        } catch {
            return [];
        }
    },
});

/** Compare "131.0.6778.204"-style versions, newest first. */
const newestFirst = (a: string, b: string): number => {
    const pa = a.split(".").map(Number);
    const pb = b.split(".").map(Number);
    for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
        const d = (pb[i] ?? 0) - (pa[i] ?? 0);
        if (d !== 0) return d;
    }
    return 0;
};

/** Puppeteer's name for this host's platform, as in its cache's
 *  `<platform>-<version>` folders, or null for one it has no builds for. */
function puppeteerPlatform(l: BrowserLookup): string | null {
    if (l.platform === "darwin") return l.arch === "arm64" ? "mac_arm" : "mac";
    if (l.platform === "linux") return l.arch === "arm64" ? "linux_arm" : "linux";
    if (l.platform === "win32") return l.arch === "ia32" ? "win32" : "win64";
    return null;
}

/** Chrome's headless shell, on PATH or in the Puppeteer cache (newest build
 *  for this OS and architecture first: a cache can hold others'). */
function findHeadlessShell(l: BrowserLookup): string | null {
    const exe = l.platform === "win32" ? "chrome-headless-shell.exe" : "chrome-headless-shell";
    const sep = l.platform === "win32" ? ";" : ":";
    for (const dir of (l.env.PATH ?? "").split(sep).filter(Boolean)) {
        if (l.exists(join(dir, exe))) return join(dir, exe);
    }
    const cache = join(l.env.PUPPETEER_CACHE_DIR ?? join(l.home, ".cache", "puppeteer"), "chrome-headless-shell");
    const host = puppeteerPlatform(l);
    const builds = l
        .list(cache)
        .filter((name) => host !== null && name.startsWith(`${host}-`))
        .map((name) => ({ name, version: name.slice(host!.length + 1) }))
        .sort((a, b) => newestFirst(a.version, b.version));
    for (const { name } of builds) {
        for (const inner of l.list(join(cache, name))) {
            const candidate = join(cache, name, inner, exe);
            if (l.exists(candidate)) return candidate;
        }
    }
    return null;
}

/**
 * The browser layout tests run in, or null (they then skip). In order:
 * `AGENTMUX_TEST_BROWSER`; Chrome's headless shell, which has no app bundle and
 * so never gets a Dock tile; then, anywhere but macOS, an installed Chrome or
 * Edge. On macOS the Chrome app is never launched: a direct launch with its
 * own profile gets a Dock tile and is kept in the Dock's recent apps
 * (docs/specs/SPEC_LAYOUT_TESTS_HEADLESS_SHELL_2026_10_09.md).
 */
export function findBrowser(lookup: BrowserLookup = realLookup()): string | null {
    const forced = lookup.env.AGENTMUX_TEST_BROWSER;
    if (forced && lookup.exists(forced)) return forced;
    const shell = findHeadlessShell(lookup);
    if (shell) return shell;
    if (lookup.platform === "darwin") return null;
    const apps = [
        "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
        "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ];
    return apps.find((c) => lookup.exists(c)) ?? null;
}

/** Why layout tests skip on a machine with no usable browser. */
export const NO_BROWSER_HINT =
    "layout tests skipped: no headless browser. Install Chrome's headless shell with " +
    "`npx @puppeteer/browsers install chrome-headless-shell@stable --path ~/.cache/puppeteer`, or set AGENTMUX_TEST_BROWSER.";

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
                // The headless shell is always headless and takes no mode.
                ...(basename(browser).startsWith("chrome-headless-shell") ? [] : ["--headless=new"]),
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
        // `close`, not `exit`: exit can come before stdout is drained, and the
        // result may be in the last of it.
        child.on("close", () => {
            const m = RESULT_RE.exec(out);
            if (!m) finish(new Error("the page produced no measurements:\n" + out.slice(0, 2000)));
        });
    });
}
