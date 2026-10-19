// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { describeOpen, findOpenLines, longestTaskIn, opensLine, parseOpenLine, summariseOpens, withLongTasks } from "./lib/agentopen.mjs";
import { mismatch } from "./compare.mjs";
import { compareRuns } from "./lib/summary.mjs";
import { generateConversation } from "./lib/conversation.mjs";

const OPEN = '[agent-open] agent="Lazo Two" block=1a2b3c4d source=mount outcome=quiet total=1341 cli=180 config=402 committed=470 history_start=520 history_read=610 history_lines=5000 parsed=627 first_screen=700 painted=812 revealed=815 quiet=1341 auth=authenticated';
const REST = "[agent-open-rest] block=1a2b3c4d outcome=done pages=3 lines=4000 bytes=900000 ms=2500";

describe("the app's open lines", () => {
    it("reads the fields: numbers as numbers, quoted values unquoted", () => {
        const p = parseOpenLine(OPEN);
        expect(p.kind).toBe("open");
        expect(p.fields).toMatchObject({ agent: "Lazo Two", block: "1a2b3c4d", source: "mount", outcome: "quiet", total: 1341, first_screen: 700, painted: 812, auth: "authenticated" });
        expect(parseOpenLine(REST)).toMatchObject({ kind: "rest", fields: { pages: 3, ms: 2500 } });
        expect(parseOpenLine("[fe] something else")).toBeNull();
    });

    it("keeps a block id that happens to be all digits as text, so the block's line is still found", () => {
        const line = OPEN.replace("1a2b3c4d", "39627044");
        expect(parseOpenLine(line).fields.block).toBe("39627044");
        expect(findOpenLines({ lines: [[1000, line]] }, "39627044-0000-1111-2222-333344445555").open.at).toBe(1000);
        expect(parseOpenLine(line).fields.total).toBe(1341);
    });

    it("finds a block's lines among others, by the block's first 8 characters", () => {
        const records = [[1000, OPEN.replace("1a2b3c4d", "ffffffff")], [2000, OPEN], [5000, REST]];
        const { open, rest } = findOpenLines({ lines: records }, "1a2b3c4d-0000-1111-2222-333344445555");
        expect(open.at).toBe(2000);
        expect(rest.at).toBe(5000);
        expect(findOpenLines(null, "x")).toEqual({ open: null, rest: null });
    });
});

describe("one open from the request", () => {
    // Asked for at 10_000; the line (logged at quiet) arrived at 12_000 and says 1341 ms since the mount, so the pane mounted at 10_659.
    const o = describeOpen({ agent: "A", kind: "regular", rpcStart: 10_000, rpcMs: 120, open: { at: 12_000, ...parseOpenLine(OPEN).fields }, rest: { at: 14_000, ...parseOpenLine(REST).fields } });

    it("counts the phases from the request, not from the mount", () => {
        expect(o.mountAfterMs).toBe(659);
        expect(o.firstScreenMs).toBe(1359); // 659 + first_screen 700
        expect(o.firstScreenPhase).toBe("first_screen");
        expect(o.paintedMs).toBe(1471);
        expect(o.revealedMs).toBe(1474);
        expect(o.quietMs).toBe(2000);
        expect(o.historyReadMs).toBe(90);
        expect(o.rest).toMatchObject({ pages: 3, doneAfterMs: 4000 });
    });

    it("uses painted on a build without the first_screen phase, and says no line when there was none", () => {
        const { first_screen, ...old } = parseOpenLine(OPEN).fields;
        const p = describeOpen({ agent: "A", kind: "regular", rpcStart: 10_000, rpcMs: 1, open: { at: 12_000, ...old }, rest: null });
        expect(p.firstScreenMs).toBe(1471);
        expect(p.firstScreenPhase).toBe("painted");
        expect(p.rest).toBeNull();
        expect(describeOpen({ agent: "A", kind: "heavy", rpcStart: 0, rpcMs: 5, open: null, rest: null })).toEqual({ agent: "A", kind: "heavy", rpcMs: 5, outcome: "no-line" });
    });

    it("sets the longest main-thread task in the wait for the first screen and in the whole open", () => {
        const tasks = [[9_000, 500], [10_200, 300], [11_000, 900], [12_500, 700]]; // [start, ms]
        expect(longestTaskIn(tasks, 10_000, 11_359)).toBe(900); // the 11_000 task overlaps the end of the wait
        expect(longestTaskIn(tasks, 10_000, 10_100)).toBe(0); // the 9_000 task ended at 9_500
        const [w] = withLongTasks([o], tasks);
        expect(w.longestTaskToFirstScreenMs).toBe(900);
        expect(w.longestTaskDuringOpenMs).toBe(900);
        expect(w.span).toBeUndefined();
    });
});

describe("the opens of a run", () => {
    const open = (kind, first, task = 0, extra = {}) => ({ kind, outcome: "quiet", firstScreenMs: first, longestTaskDuringOpenMs: task, ...extra });

    it("summarises each kind", () => {
        const s = summariseOpens([open("regular", 300, 80), open("regular", 500, 120), open("regular", 400), open("heavy", 2500, 900), { kind: "regular", outcome: "no-line" }]);
        expect(s.regular).toMatchObject({ opens: 4, noLine: 1, firstScreenMedianMs: 400, firstScreenMaxMs: 500, longestTaskMs: 120 });
        expect(s.heavy).toMatchObject({ opens: 1, firstScreenMedianMs: 2500, longestTaskMs: 900 });
        expect(opensLine(s)).toContain("regular: 4 open(s), first screen median 400 ms, worst 500 ms");
        expect(opensLine(s)).toContain("1 without a line");
    });

    it("is compared between builds as report-only rows", () => {
        const run = (first, heavy) => ({ opens: { summary: { regular: { firstScreenMedianMs: first, longestTaskMs: 100 }, heavy: { firstScreenMedianMs: heavy, longestTaskMs: 900 } } } });
        const cmp = compareRuns([run(900, 8000), run(950, 8200)], [run(300, 700), run(320, 650)]);
        expect(cmp.opens.regular.base.median).toBe(925);
        expect(cmp.opens.regular.head.median).toBe(310);
        expect(cmp.opens.heavy.pct).toBeLessThan(-90);
        expect(cmp.opens.regular.flagged).toBe(false);
        expect(compareRuns([{}], [{}]).opens.regular.base.n).toBe(0);
    });
});

describe("heavy turns in the scripted conversation", () => {
    it("leave the regular turns as they were, and add turns that only their own message plays", () => {
        const plain = generateConversation({ turns: 3, seed: 7 });
        const withHeavy = generateConversation({ turns: 3, seed: 7, heavy: { turns: 2, kb: 200 } });
        expect(withHeavy.turns.slice(0, 3)).toEqual(plain.turns);
        expect(withHeavy.turns).toHaveLength(5);
        expect(withHeavy.turns.slice(3).map((t) => t.match)).toEqual(["[heavy 1]", "[heavy 2]"]);
        expect(plain.turns.some((t) => t.match)).toBe(false);
        for (const t of withHeavy.turns.slice(3)) expect(JSON.stringify(t).length).toBeGreaterThan(200 * 1024);
    });

    it("makes a turn about as big as asked, not many times over", () => {
        const t = generateConversation({ turns: 0, seed: 1, heavy: { turns: 1, kb: 600 } }).turns[0];
        expect(JSON.stringify(t).length).toBeLessThan(900 * 1024);
    });
});

describe("which runs' opens can be compared", () => {
    const run = (opens) => ({ layout: { panes: 4 }, rounds: 2, seed: 1, settleSeconds: 5, steady: { samples: 1 }, gapSeconds: 5, platform: "win32", opens });
    const opens = (n, heavy) => ({ opens: Array.from({ length: n }, () => ({})), heavy });

    it("refuses opens of a different number of agents or a different long conversation", () => {
        expect(mismatch(run(opens(3, { turns: 1, kb: 600 })), run(opens(3, { turns: 1, kb: 600 })))).toBeNull();
        expect(mismatch(run(opens(3, null)), run(opens(4, null)))).toMatch(/different agent opens/);
        expect(mismatch(run(opens(3, { turns: 1, kb: 600 })), run(opens(3, { turns: 1, kb: 2048 })))).toMatch(/600 KB.*2048 KB/);
        expect(mismatch(run(opens(3, null)), run(opens(3, { turns: 80, kb: 600 })))).toMatch(/different agent opens/);
    });

    it("lets a run without opens be compared on everything else", () => {
        expect(mismatch(run(null), run(opens(3, null)))).toBeNull();
    });
});
