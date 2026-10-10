// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which browser layout tests run in
 * (docs/specs/SPEC_LAYOUT_TESTS_HEADLESS_SHELL_2026_10_09.md): never the Chrome
 * app on macOS, where a direct launch gets a Dock tile.
 */

import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { findBrowser, NO_BROWSER_HINT, type BrowserLookup } from "./layout-browser";

const CHROME_APP = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const HOME = "/home/u";
const CACHE = join(HOME, ".cache", "puppeteer", "chrome-headless-shell");

function lookup(over: Partial<BrowserLookup> & { files?: string[]; dirs?: Record<string, string[]> }): BrowserLookup {
    const files = new Set(over.files ?? []);
    return {
        env: over.env ?? {},
        platform: over.platform ?? "darwin",
        arch: over.arch ?? "arm64",
        home: HOME,
        exists: (p) => files.has(p),
        list: (d) => over.dirs?.[d] ?? [],
    };
}

const shellIn = (version: string, platform = "mac_arm") => ({
    dir: `${platform}-${version}`,
    path: join(CACHE, `${platform}-${version}`, `chrome-headless-shell-${platform}`, "chrome-headless-shell"),
});

describe("findBrowser", () => {
    it("takes AGENTMUX_TEST_BROWSER first", () => {
        expect(
            findBrowser(lookup({ env: { AGENTMUX_TEST_BROWSER: "/x/browser" }, files: ["/x/browser", CHROME_APP] }))
        ).toBe("/x/browser");
    });

    it("finds the headless shell on PATH", () => {
        const onPath = join("/opt/bin", "chrome-headless-shell");
        expect(findBrowser(lookup({ env: { PATH: "/opt/bin:/usr/bin" }, files: [onPath] }))).toBe(onPath);
    });

    it("finds the newest headless shell in the Puppeteer cache", () => {
        const old = shellIn("131.0.6778.204");
        const fresh = shellIn("141.0.7390.54");
        const l = lookup({
            files: [old.path, fresh.path],
            dirs: {
                [CACHE]: [old.dir, fresh.dir],
                [join(CACHE, old.dir)]: ["chrome-headless-shell-mac_arm"],
                [join(CACHE, fresh.dir)]: ["chrome-headless-shell-mac_arm"],
            },
        });
        expect(findBrowser(l)).toBe(fresh.path);
    });

    it("takes only a shell built for this OS and architecture", () => {
        const linux = shellIn("150.0.0.1", "linux");
        const mac = shellIn("131.0.6778.204", "mac_arm");
        const l = lookup({
            files: [linux.path, mac.path],
            dirs: {
                [CACHE]: [linux.dir, mac.dir],
                [join(CACHE, linux.dir)]: ["chrome-headless-shell-linux"],
                [join(CACHE, mac.dir)]: ["chrome-headless-shell-mac_arm"],
            },
        });
        expect(findBrowser(l)).toBe(mac.path);
        expect(findBrowser({ ...l, platform: "linux", arch: "x64" })).toBe(linux.path);
        expect(findBrowser({ ...l, platform: "darwin", arch: "x64" })).toBeNull();
    });

    it("on 64-bit Windows takes a win64 build first, else a win32 one (what Puppeteer installs on Windows 10 ARM)", () => {
        const exe = (dir: string, inner: string) => join(CACHE, dir, inner, "chrome-headless-shell.exe");
        const w32 = exe("win32-131.0.6778.204", "chrome-headless-shell-win32");
        const w64 = exe("win64-131.0.6778.204", "chrome-headless-shell-win64");
        const dirs = {
            [CACHE]: ["win32-131.0.6778.204", "win64-131.0.6778.204"],
            [join(CACHE, "win32-131.0.6778.204")]: ["chrome-headless-shell-win32"],
            [join(CACHE, "win64-131.0.6778.204")]: ["chrome-headless-shell-win64"],
        };
        expect(findBrowser(lookup({ platform: "win32", arch: "arm64", files: [w32], dirs }))).toBe(w32);
        expect(findBrowser(lookup({ platform: "win32", arch: "x64", files: [w32, w64], dirs }))).toBe(w64);
    });

    it("searches the default cache too when PUPPETEER_CACHE_DIR points elsewhere", () => {
        const mac = shellIn("131.0.6778.204");
        const l = lookup({
            env: { PUPPETEER_CACHE_DIR: "/custom/cache" },
            files: [mac.path],
            dirs: { [CACHE]: [mac.dir], [join(CACHE, mac.dir)]: ["chrome-headless-shell-mac_arm"] },
        });
        expect(findBrowser(l)).toBe(mac.path);
    });

    it("the install hint puts the shell where the lookup searches", () => {
        expect(NO_BROWSER_HINT).toContain("--path ~/.cache/puppeteer");
    });

    it("never launches the Chrome app on macOS: no shell means skip", () => {
        expect(findBrowser(lookup({ platform: "darwin", files: [CHROME_APP] }))).toBeNull();
    });

    it("falls back to an installed Chrome elsewhere (CI's Linux runner)", () => {
        expect(findBrowser(lookup({ platform: "linux", files: ["/usr/bin/google-chrome"] }))).toBe(
            "/usr/bin/google-chrome"
        );
    });
});
