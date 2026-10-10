// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which browser layout tests run in
 * (docs/specs/SPEC_LAYOUT_TESTS_HEADLESS_SHELL_2026_10_09.md): never the Chrome
 * app on macOS, where a direct launch gets a Dock tile.
 */

import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { findBrowser, type BrowserLookup } from "./layout-browser";

const CHROME_APP = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const HOME = "/home/u";
const CACHE = join(HOME, ".cache", "puppeteer", "chrome-headless-shell");

function lookup(over: Partial<BrowserLookup> & { files?: string[]; dirs?: Record<string, string[]> }): BrowserLookup {
    const files = new Set(over.files ?? []);
    return {
        env: over.env ?? {},
        platform: over.platform ?? "darwin",
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
        expect(
            findBrowser(lookup({ env: { PATH: "/opt/bin:/usr/bin" }, files: ["/opt/bin/chrome-headless-shell"] }))
        ).toBe("/opt/bin/chrome-headless-shell");
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

    it("never launches the Chrome app on macOS: no shell means skip", () => {
        expect(findBrowser(lookup({ platform: "darwin", files: [CHROME_APP] }))).toBeNull();
    });

    it("falls back to an installed Chrome elsewhere (CI's Linux runner)", () => {
        expect(findBrowser(lookup({ platform: "linux", files: ["/usr/bin/google-chrome"] }))).toBe(
            "/usr/bin/google-chrome"
        );
    });
});
