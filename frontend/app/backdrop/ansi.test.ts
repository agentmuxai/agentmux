// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The active theme's terminal palette: what is read, that it is read once, and that a theme change reads it again. */

import { createRoot, createEffect } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ANSI_TOKENS, readAnsi, resetThemeAnsi, themeAnsi } from "./ansi";

const root = document.documentElement;
const setAll = (value: (token: string, i: number) => string) => ANSI_TOKENS.forEach((t, i) => root.style.setProperty(t, value(t, i)));
const hexOf = (i: number) => `#${(0x102030 + i * 0x010101).toString(16)}`;
const tick = () => new Promise((r) => setTimeout(r, 0));

afterEach(() => {
    for (const t of ANSI_TOKENS) root.style.removeProperty(t);
    root.removeAttribute("data-theme");
    resetThemeAnsi();
    vi.restoreAllMocks();
});

describe("readAnsi", () => {
    it("reads the nine tokens the transcript uses, as #rrggbb", () => {
        expect(ANSI_TOKENS).toHaveLength(9);
        setAll((_, i) => hexOf(i));
        expect(readAnsi()).toEqual(Object.fromEntries(ANSI_TOKENS.map((t, i) => [t, hexOf(i)])));
    });

    it("widens #rgb, lower-cases, and leaves out a token the theme does not give as a hex", () => {
        root.style.setProperty("--term-green", "#ABC");
        root.style.setProperty("--term-blue", "rgb(1, 2, 3)");
        root.style.setProperty("--term-cyan", "teal");
        expect(readAnsi()).toEqual({ "--term-green": "#aabbcc" });
    });

    it("is empty for a document that sets none", () => {
        expect(readAnsi()).toEqual({});
    });
});

describe("themeAnsi", () => {
    it("is read once however often it is asked, so a render or a frame costs nothing", () => {
        setAll((_, i) => hexOf(i));
        const spy = vi.spyOn(window, "getComputedStyle");
        for (let i = 0; i < 50; i++) themeAnsi();
        expect(spy).toHaveBeenCalledTimes(1);
    });

    it("is read again after the theme changes, and an effect that reads it runs again", async () => {
        setAll((_, i) => hexOf(i));
        const seen: string[] = [];
        const dispose = createRoot((d) => {
            createEffect(() => seen.push(themeAnsi()["--term-green"]));
            return d;
        });
        const spy = vi.spyOn(window, "getComputedStyle");
        setAll((_, i) => hexOf(i + 40));
        root.setAttribute("data-theme", "midnight");
        await tick();
        expect(seen).toEqual([hexOf(ANSI_TOKENS.indexOf("--term-green")), hexOf(ANSI_TOKENS.indexOf("--term-green") + 40)]);
        expect(spy).toHaveBeenCalledTimes(1);
        dispose();
    });

    it("is not read, and nothing is watched, until something asks", async () => {
        const spy = vi.spyOn(window, "getComputedStyle");
        root.setAttribute("data-theme", "midnight");
        await tick();
        expect(spy).not.toHaveBeenCalled();
    });
});
