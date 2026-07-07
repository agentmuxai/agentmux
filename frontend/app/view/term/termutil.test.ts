// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { tryDeriveTermThemeFromCss } from "./termutil";

const TERM_TOKENS: Record<string, string> = {
    "--term-black": "#000000",
    "--term-red": "#cc0000",
    "--term-green": "#4e9a06",
    "--term-yellow": "#c4a000",
    "--term-blue": "#3465a4",
    "--term-magenta": "#bc3fbc",
    "--term-cyan": "#06989a",
    "--term-white": "#d0d0d0",
    "--term-bright-black": "#555753",
    "--term-bright-red": "#ef2929",
    "--term-bright-green": "#58c142",
    "--term-bright-yellow": "#fce94f",
    "--term-bright-blue": "#32afff",
    "--term-bright-magenta": "#ad7fa8",
    "--term-bright-cyan": "#34e2e2",
    "--term-bright-white": "#e7e7e7",
    "--term-gray": "#8b918a",
    "--term-cmdtext": "#ffffff",
    "--term-foreground": "#d3d7cf",
    "--term-background": "#000000",
    "--term-selection-background": "#ffffff60",
    "--term-cursor-accent": "#000000",
};

function setTermTokens(tokens: Record<string, string>, dataTheme?: string) {
    const cssText = Object.entries(tokens)
        .map(([k, v]) => `${k}: ${v};`)
        .join(" ");
    let styleEl = document.getElementById("test-term-tokens") as HTMLStyleElement | null;
    if (!styleEl) {
        styleEl = document.createElement("style");
        styleEl.id = "test-term-tokens";
        document.head.appendChild(styleEl);
    }
    styleEl.textContent = `:root { ${cssText} }`;
    if (dataTheme) {
        document.documentElement.setAttribute("data-theme", dataTheme);
    } else {
        document.documentElement.removeAttribute("data-theme");
    }
}

afterEach(() => {
    document.getElementById("test-term-tokens")?.remove();
    document.documentElement.removeAttribute("data-theme");
});

describe("tryDeriveTermThemeFromCss", () => {
    it("maps every --term-* custom property to the matching TermThemeType field", () => {
        setTermTokens(TERM_TOKENS, "dracula");
        const theme = tryDeriveTermThemeFromCss();
        expect(theme).not.toBeNull();
        expect(theme!.black).toBe("#000000");
        expect(theme!.brightWhite).toBe("#e7e7e7");
        expect(theme!.gray).toBe("#8b918a");
        expect((theme as any).cmdtext).toBe("#ffffff");
        expect(theme!.foreground).toBe("#d3d7cf");
        expect(theme!.background).toBe("#000000");
        expect(theme!.selectionBackground).toBe("#ffffff60");
        expect((theme as any).cursorAccent).toBe("#000000");
        expect(theme!["display:name"]).toBe("App theme (dracula)");
    });

    it("falls back to the foreground color for cursor (no dedicated --term-cursor token)", () => {
        setTermTokens(TERM_TOKENS);
        const theme = tryDeriveTermThemeFromCss();
        expect(theme!.cursor).toBe(theme!.foreground);
    });

    it("labels the default (no data-theme attribute) case as 'default'", () => {
        setTermTokens(TERM_TOKENS);
        const theme = tryDeriveTermThemeFromCss();
        expect(theme!["display:name"]).toBe("App theme (default)");
    });

    it("returns null when --term-background/--term-foreground aren't defined", () => {
        setTermTokens({});
        const theme = tryDeriveTermThemeFromCss();
        expect(theme).toBeNull();
    });
});
