// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A start that waits on a slow server must not look like a blank page: after a few seconds the splash says what it
// is doing (public/early-boot.js). It only adds words, and only while the splash is still up.

// @vitest-environment jsdom

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const code = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "..", "public", "early-boot.js"), "utf8");

function page(): { splash: HTMLElement; headline: HTMLElement } {
    document.body.innerHTML =
        '<div id="startup-loading"><div id="startup-loading-headline" style="display:none">Restoring session…</div></div>';
    return {
        splash: document.getElementById("startup-loading")!,
        headline: document.getElementById("startup-loading-headline")!,
    };
}

/** Run the script as the page does: a classic script, at the address the page was opened with. */
function load(search = ""): void {
    window.history.replaceState(null, "", `/${search}`);
    new Function(code)();
}

const shown = (h: HTMLElement) => h.style.display !== "none";

describe("the splash's words while a start waits", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        document.documentElement.removeAttribute("data-pool-window");
    });
    afterEach(() => {
        vi.useRealTimers();
        window.history.replaceState(null, "", "/");
    });

    it("says nothing in the first seconds: a normal start never shows the line", () => {
        const { headline } = page();
        load();
        vi.advanceTimersByTime(2999);
        expect(shown(headline)).toBe(false);
    });

    it("says it is connecting once the start has waited three seconds", () => {
        const { headline } = page();
        load();
        vi.advanceTimersByTime(3000);
        expect(shown(headline)).toBe(true);
        expect(headline.textContent).toBe("Connecting to AgentMux…");
    });

    it("says what a person can do after a longer wait", () => {
        const { headline } = page();
        load();
        vi.advanceTimersByTime(15000);
        expect(headline.textContent).toBe("Still connecting. If this stays, reload the page.");
        expect(shown(headline)).toBe(true);
    });

    it("adds nothing once the splash has gone (the start finished) or is fading out", () => {
        const first = page();
        load();
        first.splash.remove();
        expect(() => vi.advanceTimersByTime(20000)).not.toThrow();

        const second = page();
        load();
        second.splash.classList.add("fading");
        vi.advanceTimersByTime(20000);
        expect(shown(second.headline)).toBe(false);
    });

    it("leaves a pre-warmed pool window alone: it waits hidden for hours", () => {
        const { headline } = page();
        load();
        document.documentElement.setAttribute("data-pool-window", "");
        vi.advanceTimersByTime(20000);
        expect(shown(headline)).toBe(false);
    });

    it("leaves a window opened to restore a session to its own headline", () => {
        const { headline } = page();
        load("?restoring=1");
        vi.advanceTimersByTime(20000);
        expect(headline.textContent).toBe("Restoring session…");
    });
});
