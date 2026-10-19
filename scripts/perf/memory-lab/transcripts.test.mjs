// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { checkTranscripts, expectedToolUses, streamKind, transcriptsLine } from "./lib/transcripts.mjs";

const RESULT = JSON.stringify({ type: "result", subtype: "success" });
const read = (over = {}) => ({ totalLines: 816, stream: "g:agent:abc:current", gen: "x", size: 1000, last: RESULT, toolUses: 40, hashAll: 111, hashFirst: null, ...over });

describe("the tool calls the script made", () => {
    const script = {
        turns: [
            { steps: [{ thinking: "a" }, { tool: "Bash" }, { tool: "Read" }] },
            { steps: [{ tool: "Grep" }, { text: "x" }] },
            { steps: [{ tool: "Edit" }] }, // a spare turn, never played with 2 rounds
            { match: "[heavy 1]", steps: [{ tool: "Bash" }, { tool: "Bash" }, { tool: "Bash" }] },
            { match: "[heavy 2]", steps: [{ tool: "Bash" }] },
        ],
    };

    it("counts the played regular turns, and the heavy ones for the agent that played them", () => {
        expect(expectedToolUses(script, 2)).toBe(3);
        expect(expectedToolUses(script, 2, 1)).toBe(6);
        expect(expectedToolUses(script, 2, 2)).toBe(7);
        expect(expectedToolUses(script, 10)).toBe(4); // more rounds than turns: all the regular ones
    });
});

describe("the transcripts before and after", () => {
    const before = [{ agent: "t1-1", read: read() }, { agent: "t1-2", read: read({ stream: "g:agent:def:current", hashAll: 222 }) }];
    // the second reading is told how many lines the first had, and hashes those
    const same = [{ agent: "t1-1", read: read({ hashFirst: 111 }) }, { agent: "t1-2", read: read({ stream: "g:agent:def:current", hashFirst: 222 }) }];

    it("pass when nothing changes", () => {
        const c = checkTranscripts(before, same, { "t1-1": 30, "t1-2": 40 });
        expect(c.ok).toBe(true);
        expect(c.agents[0]).toMatchObject({ streamKind: "g", linesBefore: 816, linesAfter: 816, linesAdded: 0, startsWithBefore: true, lastIsResult: true, toolUses: 40, toolUsesExpected: 30 });
        expect(transcriptsLine(c)).toContain("2 agents");
    });

    it("pass when a transcript grew and kept every line it had (a reopen restarts the CLI, which plays a turn)", () => {
        const grown = [{ agent: "t1-1", read: read({ totalLines: 1631, hashFirst: 111, toolUses: 112 }) }, same[1]];
        const c = checkTranscripts(before, grown, { "t1-1": 30, "t1-2": 40 });
        expect(c.ok).toBe(true);
        expect(c.agents[0]).toMatchObject({ linesAdded: 815, startsWithBefore: true });
        expect(transcriptsLine(c)).toContain("1 grew");
    });

    it("name each thing that went wrong", () => {
        const after = [
            { agent: "t1-1", read: read({ totalLines: 800, hashFirst: 111 }) },
            { agent: "t1-2", read: read({ stream: "b:block:1", last: JSON.stringify({ type: "assistant" }), toolUses: 3, hashFirst: 999 }) },
        ];
        const c = checkTranscripts(before, after, { "t1-1": 30, "t1-2": 40 });
        expect(c.ok).toBe(false);
        expect(c.problems.join("\n")).toMatch(/t1-1: 816 lines before, 800 after: lines were lost/);
        expect(c.problems.join("\n")).toMatch(/t1-2: the lines it had before were changed/);
        expect(c.problems.join("\n")).toMatch(/t1-2: the stream changed \(g:agent:def:current to b:block:1\)/);
        expect(c.problems.join("\n")).toMatch(/t1-2: the newest line is not the end of a turn/);
        expect(c.problems.join("\n")).toMatch(/t1-2: 3 tool calls in the transcript, the script made 40/);
        expect(transcriptsLine(c)).toContain("TRANSCRIPT PROBLEM");
    });

    it("catches a newest line that changed with no line added", () => {
        const after = [{ agent: "t1-1", read: read({ hashFirst: 111, last: RESULT.replace("success", "error_x") }) }, same[1]];
        expect(checkTranscripts(before, after, {}).problems.join()).toMatch(/newest line changed with no line added/);
    });

    it("do not compare line by line, or count tool calls, a transcript too big to read whole; and report an unreadable one", () => {
        const big = (b) => ({ ...b, read: { ...b.read, toolUses: null, hashAll: null, hashFirst: null } });
        const c = checkTranscripts(before.map(big), same.map(big), { "t1-1": 30, "t1-2": 40 });
        expect(c.ok).toBe(true);
        expect(c.agents[0].startsWithBefore).toBeNull();
        expect(transcriptsLine(c)).toContain("too big");
        const bad = checkTranscripts(before, [{ agent: "t1-1", read: { error: "boom" } }, same[1]], {});
        expect(bad.problems[0]).toMatch(/t1-1: could not read the transcript \(boom\)/);
    });

    it("says which stream", () => {
        expect(streamKind("g:agent:abc:current")).toBe("g");
        expect(streamKind("b:x")).toBe("b");
        expect(streamKind(null)).toBeNull();
    });
});
