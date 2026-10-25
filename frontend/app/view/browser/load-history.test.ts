// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import {
    estimatedLoadMs,
    LOADS_PER_ORIGIN,
    MAX_ORIGINS,
    medianLoadMs,
    originOf,
    recordLoad,
    withLoad,
    type LoadHistory,
} from "./load-history";

afterEach(() => localStorage.clear());

describe("originOf", () => {
    it("is the scheme, host and port of a web address", () => {
        expect(originOf("https://github.com/a/b/pull/1?x=1#y")).toBe("https://github.com");
        expect(originOf("http://localhost:5173/x")).toBe("http://localhost:5173");
    });
    it("is nothing for an address that isn't a web page", () => {
        expect(originOf("about:blank")).toBeUndefined();
        expect(originOf("data:text/html,x")).toBeUndefined();
        expect(originOf("not a url")).toBeUndefined();
        expect(originOf("")).toBeUndefined();
    });
});

describe("withLoad", () => {
    it("keeps the newest few loads of an origin", () => {
        let h: LoadHistory = {};
        for (const ms of [100, 200, 300, 400, 500, 600, 700]) h = withLoad(h, "https://a.test", ms);
        expect(h["https://a.test"]).toEqual([300, 400, 500, 600, 700]);
        expect(h["https://a.test"]).toHaveLength(LOADS_PER_ORIGIN);
    });
    it("ignores a time that can't be a page load", () => {
        const h: LoadHistory = { "https://a.test": [500] };
        expect(withLoad(h, "https://a.test", 0)).toBe(h);
        expect(withLoad(h, "https://a.test", -5)).toBe(h);
        expect(withLoad(h, "https://a.test", Number.NaN)).toBe(h);
        expect(withLoad(h, "https://a.test", 10 * 60_000)).toBe(h);
    });
    it("does not change the history it was given", () => {
        const h: LoadHistory = { "https://a.test": [500] };
        withLoad(h, "https://a.test", 600);
        expect(h).toEqual({ "https://a.test": [500] });
    });
    it("drops the least recently loaded origins past the cap", () => {
        let h: LoadHistory = {};
        for (let i = 0; i < MAX_ORIGINS + 3; i++) h = withLoad(h, `https://site${i}.test`, 500);
        expect(Object.keys(h)).toHaveLength(MAX_ORIGINS);
        expect(h["https://site0.test"]).toBeUndefined();
        expect(h[`https://site${MAX_ORIGINS + 2}.test`]).toEqual([500]);
        // Loading an old origin again makes it the newest.
        h = withLoad(h, "https://site3.test", 500);
        h = withLoad(h, "https://another.test", 500);
        expect(h["https://site3.test"]).toEqual([500, 500]);
        expect(h["https://site4.test"]).toBeUndefined();
    });
});

describe("medianLoadMs", () => {
    it("is the middle load, so one slow load doesn't move it", () => {
        expect(medianLoadMs({ o: [900, 1000, 30_000] }, "o")).toBe(1000);
        expect(medianLoadMs({ o: [300, 900, 1000, 1100, 30_000] }, "o")).toBe(1000);
    });
    it("is the mean of the two middle loads when there is an even number", () => {
        expect(medianLoadMs({ o: [1000, 2000] }, "o")).toBe(1500);
    });
    it("is nothing without usable loads", () => {
        expect(medianLoadMs({}, "o")).toBeUndefined();
        expect(medianLoadMs({ o: [] }, "o")).toBeUndefined();
        expect(medianLoadMs({ o: "x" as unknown as number[] }, "o")).toBeUndefined();
        expect(medianLoadMs({ o: ["x" as unknown as number] }, "o")).toBeUndefined();
    });
});

describe("the stored history", () => {
    it("estimates a page from its site's recent loads, across pages of the site", () => {
        recordLoad("https://github.com/a/b/pull/1", 1200);
        recordLoad("https://github.com/a/b/issues/2", 800);
        recordLoad("https://github.com/c", 1000);
        recordLoad("https://example.com/", 3000);
        expect(estimatedLoadMs("https://github.com/anything")).toBe(1000);
        expect(estimatedLoadMs("https://example.com/")).toBe(3000);
        expect(estimatedLoadMs("https://unseen.test/")).toBeUndefined();
        expect(estimatedLoadMs("about:blank")).toBeUndefined();
    });
    it("survives a damaged store", () => {
        localStorage.setItem("agentmux:browser-load-history", "{not json");
        expect(estimatedLoadMs("https://github.com/")).toBeUndefined();
        recordLoad("https://github.com/", 900);
        expect(estimatedLoadMs("https://github.com/")).toBe(900);
        localStorage.setItem("agentmux:browser-load-history", "[1,2]");
        expect(estimatedLoadMs("https://github.com/")).toBeUndefined();
    });
});
