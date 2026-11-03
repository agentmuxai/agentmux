// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { GESTURE_WINDOW_MS, judgeSample, judgeTransition, parseArgs, parseTransition, summarize } from "./follow-soak.mjs";

const line = (text) => ["[scroll-follow]", ...text.split(" ")];

describe("follow-soak parseArgs", () => {
    it("defaults to the spec's acceptance run and refuses the production port", () => {
        const o = parseArgs([]);
        expect(o.minutes).toBe(30);
        expect(o.port).toBe(9223);
        expect(o.panes).toEqual([]);
        expect(o.strict).toBe(false);
    });
    it("reads the flags", () => {
        const o = parseArgs(["--minutes", "2", "--panes", "a,b", "--strict", "--seed", "7"]);
        expect([o.minutes, o.panes, o.strict, o.seed]).toEqual([2, ["a", "b"], true, 7]);
    });
    it("rejects a negative number", () => {
        expect(() => parseArgs(["--minutes", "-1"])).toThrow(/non-negative/);
    });
});

describe("parseTransition", () => {
    it("reads the page's transition line", () => {
        expect(parseTransition(line("pane=abc1234 following→detached cause=user-scroll gap=412px"))).toEqual({
            pane: "abc1234",
            from: "following",
            to: "detached",
            cause: "user-scroll",
        });
    });
    it("ignores other lines, the mount line included", () => {
        expect(parseTransition(["hello"])).toBeNull();
        expect(parseTransition(line("pane=abc1234 mount following"))).toBeNull();
    });
});

describe("judgeTransition", () => {
    const drop = (cause) => ({ pane: "abc1234", from: "following", to: "detached", cause });
    it("accepts a drop caused by our own wheel just before it", () => {
        expect(judgeTransition(drop("user-escape:wheel"), 10_000, 9_500)).toBeNull();
    });
    it("fails a drop with no gesture of ours", () => {
        expect(judgeTransition(drop("user-scroll"), 10_000, undefined)).toMatch(/dropped out of following/);
        expect(judgeTransition(drop("user-scroll"), 10_000, 10_000 - GESTURE_WINDOW_MS - 1)).toMatch(/no user gesture/);
    });
    it("fails a drop whose cause is not a user's, even right after a gesture", () => {
        expect(judgeTransition(drop("external"), 10_000, 9_900)).toMatch(/cause=external/);
    });
    it("lets a pane start following again", () => {
        expect(judgeTransition({ pane: "p", from: "detached", to: "following", cause: "reached-bottom" }, 1, undefined)).toBeNull();
    });
});

describe("judgeSample", () => {
    const sample = (over) => ({ blockId: "abc1234def", found: true, visible: true, covered: false, state: "following", gap: 0, ...over });
    it("passes a following pane at the bottom", () => {
        expect(judgeSample(sample({ gap: 1 }))).toBeNull();
    });
    it("fails a following pane above the bottom", () => {
        expect(judgeSample(sample({ gap: 40.2 }))).toMatch(/following but 40\.2 px above the bottom/);
    });
    it("does not read a detached, hidden or covered pane", () => {
        expect(judgeSample(sample({ state: "detached", gap: 500 }))).toBeNull();
        expect(judgeSample(sample({ visible: false, gap: 500 }))).toBeNull();
        expect(judgeSample(sample({ covered: true, gap: 500 }))).toBeNull();
        expect(judgeSample({ found: false })).toBeNull();
    });
});

describe("summarize", () => {
    const base = { minutes: 30, panes: 3, samples: 5400, transitions: 12, actions: { click: 4, wheel: 2 }, detachChecks: { pass: 2, total: 2 }, refollowChecks: { pass: 2, total: 2 } };
    it("says PASS with no violations", () => {
        expect(summarize({ ...base, violations: [] })).toMatch(/PASS: no scroll-follow violations/);
    });
    it("lists violations and counts them by kind", () => {
        const text = summarize({ ...base, violations: [{ kind: "not-at-bottom", text: "abc is following but 9 px above the bottom", atMs: 61_000 }] });
        expect(text).toMatch(/VIOLATION \[not-at-bottom\] t=61s/);
        expect(text).toMatch(/FAIL: 1 violation\(s\) \{"not-at-bottom":1\}/);
    });
});
