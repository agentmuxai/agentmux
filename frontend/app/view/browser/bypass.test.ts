// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { minutesLabel, originCovered, pageOrigin, siteLabel } from "./bypass";

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
