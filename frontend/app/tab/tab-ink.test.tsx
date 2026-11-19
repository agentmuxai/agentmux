// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";

import { currentTabInk, parseInkToken } from "./tab-ink";

describe("parseInkToken", () => {
    it("reads a six-digit hex and an rgb() as #rrggbb", () => {
        expect(parseInkToken(" #F7F7F7 ")).toBe("#f7f7f7");
        expect(parseInkToken("rgb(195, 200, 194)")).toBe("#c3c8c2");
    });

    it("gives nothing for any other form", () => {
        for (const raw of ["", undefined, null, "#fff", "var(--x)", "rgba(1, 2, 3, 0.5)", "hsl(0, 0%, 50%)"]) {
            expect(parseInkToken(raw as string | undefined)).toBeUndefined();
        }
    });
});

describe("currentTabInk", () => {
    afterEach(() => {
        document.documentElement.removeAttribute("style");
    });

    it("reads the theme's main and secondary text from the root", () => {
        document.documentElement.style.setProperty("--main-text-color", "#c0caf5");
        document.documentElement.style.setProperty("--secondary-text-color", "rgb(154, 165, 206)");
        expect(currentTabInk()).toEqual({ selected: "#c0caf5", unselected: "#9aa5ce" });
    });

    it("gives nothing when a token is not a plain colour, so the default theme's text is used", () => {
        document.documentElement.style.setProperty("--main-text-color", "#c0caf5");
        document.documentElement.style.setProperty("--secondary-text-color", "var(--nope)");
        expect(currentTabInk()).toBeUndefined();
    });
});
