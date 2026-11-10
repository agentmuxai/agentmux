// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { hasEnterSpacePower, hasTerminalPower, hasUploadsPower, leftLabel, minutesLabel, originCovered, pageOrigin, powersNote, siteLabel } from "./bypass";

describe("bypass sites", () => {
    it("reads a page's site, and none for pages without one", () => {
        expect(pageOrigin("https://github.com/agentmuxai/agentmux/pull/185")).toBe("https://github.com");
        expect(pageOrigin("http://localhost:3000/x")).toBe("http://localhost:3000");
        expect(pageOrigin("about:blank")).toBeUndefined();
        expect(pageOrigin("data:text/html,hi")).toBeUndefined();
        expect(pageOrigin("not a url")).toBeUndefined();
        expect(pageOrigin(undefined)).toBeUndefined();
    });

    it("covers the granted sites only, as srv does", () => {
        const origins = ["https://github.com", "https://*.example.com"];
        expect(originCovered(origins, "https://github.com/a")).toBe(true);
        expect(originCovered(origins, "https://example.com/")).toBe(true);
        expect(originCovered(origins, "https://docs.example.com/x")).toBe(true);
        expect(originCovered(origins, "https://accounts.google.com/")).toBe(false);
        expect(originCovered(origins, "http://github.com/")).toBe(false);
        expect(originCovered(origins, "https://notexample.com/")).toBe(false);
        expect(originCovered(origins, "about:blank")).toBe(false);
    });

    it("labels durations and sites", () => {
        expect(minutesLabel(15)).toBe("15 min");
        expect(minutesLabel(60)).toBe("1 hour");
        expect(minutesLabel(240)).toBe("4 hours");
        expect(siteLabel("https://github.com")).toBe("github.com");
        expect(siteLabel("http://localhost:3000")).toBe("http://localhost:3000");
    });
});

describe("hasTerminalPower", () => {
    it("is true only when the grant lists the terminal", () => {
        expect(hasTerminalPower(["clicks", "terminal"])).toBe(true);
        expect(hasTerminalPower(["clicks"])).toBe(false);
        expect(hasTerminalPower(undefined)).toBe(false);
    });
});

describe("the Enter and Space power", () => {
    it("is its own power, and the strip says it beside the terminal's", () => {
        expect(hasEnterSpacePower(["clicks", "enter_space"])).toBe(true);
        expect(hasEnterSpacePower(["clicks", "terminal"])).toBe(false);
        expect(hasEnterSpacePower(undefined)).toBe(false);
        expect(powersNote(["clicks"])).toBe("");
        expect(powersNote(["clicks", "enter_space"])).toBe(" and it presses Enter and Space, each as the click it is");
        expect(powersNote(["clicks", "terminal_ask", "enter_space"])).toBe(
            " and it types in the terminal, asking you before each line runs, and it presses Enter and Space, each as the click it is"
        );
    });
});

describe("the uploads power", () => {
    it("is its own power, and the strip says it last", () => {
        expect(hasUploadsPower(["clicks", "uploads"])).toBe(true);
        expect(hasUploadsPower(["clicks", "enter_space"])).toBe(false);
        expect(hasUploadsPower(undefined)).toBe(false);
        expect(powersNote(["clicks", "uploads"])).toBe(" and it fills a file chooser the page opens, from files in its workspace");
        expect(powersNote(["clicks", "terminal", "enter_space", "uploads"])).toBe(
            " and it runs commands in the terminal, and it presses Enter and Space, each as the click it is, and it fills a file chooser the page opens, from files in its workspace"
        );
    });
});

describe("the strip's time note", () => {
    it("counts minutes for a timed grant and says always for a standing one", () => {
        expect(leftLabel(false, 47)).toBe("47 min left");
        expect(leftLabel(undefined, 3)).toBe("3 min left");
        expect(leftLabel(true, 99999)).toBe("always, until you revoke it in Settings");
    });
});
