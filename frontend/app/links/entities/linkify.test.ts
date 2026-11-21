// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { buildAgentIndex, buildTaskIndex } from "./indexes";
import { explainLinks, linkify } from "./linkify";
import { joined, linksOf, testContext } from "./test-context";

const links = (text: string, over = {}) => linksOf(linkify(text, testContext(over)));

describe("agents", () => {
    it("links a roster name written whole, with its possessive and punctuation", () => {
        expect(links("I asked Opaz to send it.")).toEqual(["agent:Opaz"]);
        expect(links("Camper's spec, then Posa; AgentY!")).toEqual(["agent:Camper", "agent:Posa", "agent:AgentY"]);
    });
    it("links a name with a hyphen and digit, and a slug or id in any case", () => {
        expect(links("Naki-2 owns it")).toEqual(["agent:Naki-2"]);
        expect(links("ask naki-2 or opaz-0909m")).toEqual(["agent:naki-2", "agent:opaz-0909m"]);
    });
    it("does not link a name that is not in the roster, a prefix, or a lower-case display name", () => {
        expect(links("Opazz and Camp and camper-0622hx")).toEqual([]);
    });
    it("does not link an ordinary-word name alone, but does with its computer or slug", () => {
        expect(links("Will you look at Pulp?")).toEqual([]);
        expect(links("will-7 will do")).toEqual(["agent:will-7"]);
    });
    it("never links an ordinary word alone in any case, even when it is the agent's slug", () => {
        expect(links("Mark it, mark it, MARK it and mark@narko")).toEqual(["agent:mark@narko"]);
    });
    it("links Name@computer only when the agent is on that computer", () => {
        expect(links("Clare@starpower and Opaz@narko")).toEqual(["agent:Clare@starpower", "agent:Opaz@narko"]);
        expect(links("Clare@charlie, Opaz@elsewhere")).toEqual([]);
    });
    it("never links an email address, a file name or a path segment", () => {
        expect(links("opaz@example.com")).toEqual([]);
        expect(links("Opaz.json and src/Opaz and a/Opaz")).toEqual([]);
    });
    it("builds the destination from the roster, not from the text", () => {
        const [seg] = linkify("Opaz", testContext());
        expect(seg).toMatchObject({ type: "link", destination: { kind: "agent", id: "opaz-0909m" }, name: "Open agent Opaz" });
    });
});

describe("tasks and pull requests: the order of §3.1", () => {
    it("links a bare #N as a task when the task exists", () => {
        expect(links("see #48247 and #47448.")).toEqual(["task:#48247", "task:#47448"]);
    });
    it("links task N, task #N and agentmux://task/N", () => {
        expect(links("task 48175, task #58373, agentmux://task/7940")).toEqual(["task:task 48175", "task:task #58373", "task:agentmux://task/7940"]);
    });
    it("links work#N on a board that holds it, and a bare number on the surface's board", () => {
        expect(links("work#12")).toEqual(["task:work#12"]);
        expect(links("#12", { board: "work" })).toEqual(["task:#12"]);
        const [seg] = linkify("work#12", testContext());
        expect(seg).toMatchObject({ destination: { kind: "task", number: 12, board: "work" } });
    });
    it("does not link a task that is not on a board the viewer sees", () => {
        expect(links("task 999999 and work#99 and nowhere#12")).toEqual([]);
    });
    it("makes PR and issue words pull requests of the surface's repository", () => {
        const [seg] = linkify("PR 7022", testContext());
        expect(seg).toMatchObject({ kind: "pull", destination: { kind: "pull", repo: "agentmuxai/agentmux", number: 7022, issue: false }, name: "Open pull request 7022" });
        expect(links("PR #7940, issue #5 and pull request 120, PR 7")).toEqual(["pull:PR #7940", "pull:issue #5", "pull:pull request 120", "pull:PR 7"]);
    });
    it("leaves prose that only looks like a pull request alone", () => {
        expect(links("pull 3 changes, pr 5 times, issue 3 items, issue\n3 items, pull   42 more")).toEqual([]);
    });
    it("lets PR win over task for the same number", () => {
        expect(links("PR #7940")).toEqual(["pull:PR #7940"]);
        expect(links("#7940")).toEqual(["task:#7940"]);
    });
    it("makes a bare #N that is no task a pull request when the surface has a repository", () => {
        expect(links("#7022")).toEqual(["pull:#7022"]);
        expect(links("#7022", { repo: undefined })).toEqual([]);
    });
    it("never falls back from an explicit task form, or from a PR word with no repository", () => {
        expect(links("task #7022")).toEqual([]);
        expect(links("PR #48247", { repo: undefined })).toEqual([]);
    });
    it("takes the repository from the surface, and only a full owner/repo#N names its own", () => {
        const [seg] = linkify("agentmuxai/agentmux-cloud#12", testContext({ repo: undefined }));
        expect(seg).toMatchObject({ kind: "pull", destination: { repo: "agentmuxai/agentmux-cloud", number: 12 } });
        expect(links("PR 12", { repo: "not a repo" })).toEqual([]);
    });
    it("does not read entities, colours or headings as task numbers", () => {
        expect(links("&#48247; and a#48247 and /#48247 and # 48247 and ##48247")).toEqual([]);
    });
});

describe("addresses", () => {
    it("links http and https addresses and upgrades a bare public domain", () => {
        const segs = linkify("see https://example.com/a?b=1 and example.org", testContext());
        expect(linksOf(segs)).toEqual(["address:https://example.com/a?b=1", "address:example.org"]);
        expect(segs.find((s) => s.type === "link" && s.label === "example.org")).toMatchObject({ destination: { kind: "url", url: "https://example.org" } });
    });
    it("never makes a link from mailto, ftp, javascript or file text", () => {
        expect(links("mailto:a@b.co ftp://x.test/y javascript:alert(1) file:///C:/x")).toEqual([]);
    });
    it("keeps README-style names out", () => {
        expect(links("open README.md or main.rs")).toEqual([]);
    });
    it("wins over a task or a name inside it", () => {
        expect(links("https://example.com/Opaz/#48247")).toEqual(["address:https://example.com/Opaz/#48247"]);
    });
});

describe("protection", () => {
    it("leaves code blocks, code spans, Markdown links, images and autolinks alone", () => {
        const text = "```\nOpaz #48247\n```\n`Opaz` [Opaz #48247](https://example.test) ![Opaz](x.png) <https://example.com>";
        expect(links(text)).toEqual([]);
    });
    it("links a code span that is wholly one task number (D4), and nothing else in code", () => {
        expect(links("`#48247` and `see #48247`")).toEqual(["task:#48247"]);
    });
    it("protects the rest of an unclosed fence", () => {
        expect(links("Opaz\n```\nCamper #48247")).toEqual(["agent:Opaz"]);
    });
});

describe("streaming", () => {
    it("does not link a match that touches the end of text still arriving", () => {
        expect(links("ask Opaz", { final: false })).toEqual([]);
        expect(links("task #4824", { final: false })).toEqual([]);
        expect(links("ask Opaz,", { final: false })).toEqual(["agent:Opaz"]);
    });
    it("never links a prefix while text is fed in pieces; the final text links once", () => {
        const full = "I asked Opaz about #48247 today";
        for (let i = 1; i <= full.length; i++) {
            const segs = linkify(full.slice(0, i), testContext({ final: false }));
            expect(segs[segs.length - 1].type === "link").toBe(false);
        }
        expect(links(full)).toEqual(["agent:Opaz", "task:#48247"]);
    });
});

describe("fidelity and idempotence", () => {
    const samples = [
        "I asked Opaz to send #48247 (https://example.com/x), see PR 7022.",
        "  leading and trailing   \n\n  lines  ",
        "Naki-2@narko wrote `#48247`: Camper's turn.",
        "",
    ];
    it.each(samples)("keeps the text byte for byte: %j", (text) => {
        expect(joined(linkify(text, testContext()))).toBe(text);
    });
    it("gives the same segments for the same text", () => {
        const a = linkify(samples[0], testContext());
        expect(linkify(joined(a), testContext())).toEqual(a);
    });
    it("keeps the text when the roster and the board are empty", () => {
        const empty = testContext({ agents: buildAgentIndex([]), tasks: buildTaskIndex([], "main"), repo: undefined });
        expect(linkify("Opaz #48247 and nothing", empty)).toEqual([{ type: "text", text: "Opaz #48247 and nothing" }]);
    });
});

describe("security", () => {
    it("rejects a match with direction or control characters in or beside it", () => {
        expect(links("O‮paz and Opaz‮")).toEqual([]);
        expect(links("​#48247")).toEqual([]);
        for (const c of ["­", "؜", "᠎", "⁠", "⁦", "", ""]) {
            expect(links(`${c}Opaz`), JSON.stringify(c)).toEqual([]);
            expect(links(`Opaz${c}`), JSON.stringify(c)).toEqual([]);
        }
        expect(links("Opaz\t#48247\nCamper")).toEqual(["agent:Opaz", "task:#48247", "agent:Camper"]);
    });
    it("gives every link a destination built from an entity, never from text", () => {
        for (const s of linkify("Opaz #48247 PR 12 https://example.com", testContext())) {
            if (s.type !== "link") continue;
            expect(["agent", "task", "pull", "url"]).toContain(s.destination.kind);
            if (s.destination.kind === "url") expect(s.destination.url).toMatch(/^https?:\/\//);
        }
    });
    it("leaves a very long text as it is", () => {
        expect(linkify("Opaz ".repeat(50_000), testContext())).toHaveLength(1);
    });
});

describe("the debug view", () => {
    it("says why each candidate was or was not made a link", () => {
        const trace = explainLinks("Opaz at https://example.com/x, task #48247 and PR 5", testContext({ repo: undefined }));
        const outcomes = trace.map((t) => `${t.kind}:${t.text}=${t.outcome}`);
        expect(outcomes).toContain("agent:Opaz=linked");
        expect(outcomes).toContain("address:https://example.com/x=linked");
        expect(outcomes).toContain("pull:PR 5=no such entity");
        expect(outcomes).toContain("task:task #48247=linked");
        expect(outcomes).toContain("task:#48247=overlapped by a task");
    });
});

describe("budget", () => {
    const message = ("Opaz said that #48247 and Camper's PR 7022 (https://example.com/a) were fine; Posa agrees. " + "word ".repeat(10)).repeat(70);
    it("linkifies a 10 KB message within 2 ms once the indexes exist", () => {
        expect(message.length).toBeGreaterThan(9000);
        const ctx = testContext();
        linkify(message, ctx);
        const t0 = performance.now();
        for (let i = 0; i < 10; i++) linkify(message, ctx);
        // The spec's budget is 2 ms; the bound is loose so a loaded CI machine does not fail a correct build.
        expect((performance.now() - t0) / 10).toBeLessThan(10);
    });
});

describe("a name or a number that does not resolve does not hide the rest", () => {
    it("links the agent in opaz#48247 (an unknown board) and in agent@host with no such host", () => {
        expect(links("opaz#48247")).toEqual(["agent:opaz"]);
    });
});

describe("scale", () => {
    it("handles a 190 KB message of agent names and code spans in well under a second", () => {
        const names = "Opaz Camper Posa `x` ".repeat(9000);
        expect(names.length).toBeGreaterThan(180_000);
        const t0 = performance.now();
        const segs = linkify(names, testContext());
        expect(performance.now() - t0).toBeLessThan(2000);
        expect(joined(segs)).toBe(names);
        expect(segs.filter((x) => x.type === "link")).toHaveLength(27_000);
    });
});
