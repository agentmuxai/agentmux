// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The corpus gate (spec §5): messages and task cards of the kind agents write, each with the links a person would
 * expect. No false link on a labelled negative (precision 100 percent) and recall of 90 percent or better on the
 * labelled positives. Every new kind adds its own cases here.
 */

import { describe, expect, it } from "vitest";
import { linkify } from "./linkify";
import { linksOf, testContext } from "./test-context";

type Case = { text: string; expect: string[] };

const POSITIVES: Case[] = [
    { text: "I asked Opaz to send the #58373 audit table as an attachment.", expect: ["agent:Opaz", "task:#58373"] },
    { text: "Camper's deep-links spec is #48247's dependency; Posa has the Windows side.", expect: ["agent:Camper", "task:#48247", "agent:Posa"] },
    { text: "PR 7940 fixes #48175, waiting on Clare@starpower.", expect: ["pull:PR 7940", "task:#48175", "agent:Clare@starpower"] },
    { text: "Done: https://example.com/docs/page. Next is task 47448.", expect: ["address:https://example.com/docs/page", "task:task 47448"] },
    { text: "Reported to AgentY and Naki-2 (see agentmux://task/48175).", expect: ["agent:AgentY", "agent:Naki-2", "task:agentmux://task/48175"] },
    { text: "Merged PR #7940, and agentmuxai/agentmux-cloud#33 is next.", expect: ["pull:PR #7940", "pull:agentmuxai/agentmux-cloud#33"] },
    { text: "- [ ] ask opaz-0909m\n- [x] close #7940", expect: ["agent:opaz-0909m", "task:#7940"] },
    { text: "Moved to work#12 per Camper.", expect: ["task:work#12", "agent:Camper"] },
    { text: "Another project: other/project#99, and our own #8123.", expect: ["pull:other/project#99", "pull:#8123"] },
];

const NEGATIVES: string[] = [
    "Will you review it? Mark it done and move on, Pulp fiction aside.",
    "Version 1.2.3 shipped at 10:30 on 2026-11-20; build 4567 passed.",
    "Heading # 12 and an HTML entity &#48247; and a colour #fff or #123abc.",
    "Email opaz@example.com and see src/Opaz/index.ts and docs/Camper.md.",
    "Open README.md, main.rs, setup.py and Cargo.toml.",
    "Run `cargo test` and `git log #48247x` then:\n```\nOpaz #48247 https://example.com\n```",
    "Not links: javascript:alert(1), file:///C:/Windows, ftp://host/x, mailto:a@b.test",
    "Step 3 of 7, item #3 in the list, issue-tracker, the pr-review bot, task-based work.",
    "Near misses: opazz, Campers, Posas, task-based, #3, #42.",
];

describe("corpus: positives", () => {
    it("meets 90 percent recall and gets every link in its place", () => {
        let expected = 0;
        let found = 0;
        for (const c of POSITIVES) {
            const got = linksOf(linkify(c.text, testContext()));
            expected += c.expect.length;
            found += c.expect.filter((e) => got.includes(e)).length;
            expect(got.filter((g) => !c.expect.includes(g))).toEqual([]);
        }
        expect(found / expected).toBeGreaterThanOrEqual(0.9);
    });
});

describe("corpus: negatives", () => {
    it.each(NEGATIVES)("makes no false link: %j", (text) => {
        expect(linksOf(linkify(text, testContext()))).toEqual([]);
    });
});
