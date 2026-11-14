// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The review bot and CI on the map: an event becomes a line, the lines are kept per pull request, and make the two nodes. */

import { describe, expect, it } from "vitest";
import type { ChecksFlowEvent } from "@/types/rpc/ChecksFlowEvent";
import type { ReviewFlowEvent } from "@/types/rpc/ReviewFlowEvent";
import { flowEndResolver, flowsAt, MAX_PR_LINES, PR_FRESH_MS, PR_SHOWN_MS, withFlow, withoutPr, type AtlasFlow } from "./atlas-flow";
import type { AtlasAgent, AtlasMachine } from "./atlas-graph";
import { CHECKS_NODE, checksFlow, prKey, prServices, REVIEW_NODE, reviewFlow } from "./atlas-pr";

const review = (over: Partial<ReviewFlowEvent> = {}): ReviewFlowEvent => ({
    id: "inj-1",
    at_ms: 1_000,
    agent: "ann",
    pr: { repo_id: 9, number: 64 },
    state: "approved",
    by: "bot",
    ...over,
});

const checks = (over: Partial<ChecksFlowEvent> = {}): ChecksFlowEvent => ({
    id: "inj-2",
    at_ms: 1_000,
    agent: "ann",
    pr: { repo_id: 9, number: 64 },
    state: "passing",
    ...over,
});

const keep = (events: (AtlasFlow | null)[], now: number) =>
    events.reduce<readonly AtlasFlow[]>((flows, flow) => (flow ? withFlow(flows, flow, now) : flows), []);

describe("reviewFlow and checksFlow", () => {
    it("draw from the service's node to the pull request's own agent, one of this instance's", () => {
        expect(reviewFlow(review())).toMatchObject({
            id: "review:inj-1",
            kind: "review",
            from: { node: REVIEW_NODE },
            to: { agent: "ann", local: true },
            label: "#64",
            pr: "9/64",
            state: "approved",
            atMs: 1_000,
        });
        expect(checksFlow(checks())).toMatchObject({ id: "checks:inj-2", kind: "checks", from: { node: CHECKS_NODE }, label: "#64" });
    });

    it("read as passed, failed or neither, and a state this build does not know as neither", () => {
        expect(reviewFlow(review())!.tone).toBe("passed");
        expect(reviewFlow(review({ state: "changes_requested" }))!.tone).toBe("failed");
        expect(reviewFlow(review({ state: "commented" }))!.tone).toBe("neutral");
        expect(reviewFlow(review({ state: "requested" }))!.tone).toBe("neutral");
        expect(checksFlow(checks())!.tone).toBe("passed");
        expect(checksFlow(checks({ state: "failing" }))!.tone).toBe("failed");
        expect(checksFlow(checks({ state: "pending" }))!.tone).toBe("neutral");
    });

    it("say who reviewed when it was not the bot", () => {
        expect(reviewFlow(review({ by: "codex" }))!.label).toBe("#64 Codex");
        expect(reviewFlow(review({ by: "person" }))!.label).toBe("#64 person");
    });

    it("draw nothing for an event with no agent or no pull request number", () => {
        expect(reviewFlow(review({ agent: "" }))).toBeNull();
        expect(checksFlow(checks({ pr: { repo_id: 9, number: 0 } }))).toBeNull();
        expect(prKey(undefined)).toBeNull();
    });
});

describe("a pull request's lines on show", () => {
    it("keep the newest review and the newest checks of each pull request", () => {
        const flows = keep(
            [
                reviewFlow(review({ id: "a", state: "changes_requested" })),
                checksFlow(checks({ id: "b", state: "failing" })),
                reviewFlow(review({ id: "c", at_ms: 2_000 })),
                reviewFlow(review({ id: "d", pr: { repo_id: 9, number: 65 } })),
                // The same number in another repository is another pull request.
                reviewFlow(review({ id: "e", pr: { repo_id: 10, number: 64 } })),
            ],
            3_000
        );
        expect(flows.map((f) => `${f.id} ${f.state}`)).toEqual([
            "checks:b failing",
            "review:c approved",
            "review:d approved",
            "review:e approved",
        ]);
    });

    it("are bright at first, then dim until the pull request ends, and go by themselves only after hours", () => {
        const flows = keep([reviewFlow(review())], 1_000);
        expect(flowsAt(flows, 1_000 + PR_FRESH_MS - 1)).toBe(flows);
        const later = flowsAt(flows, 1_000 + PR_FRESH_MS);
        expect(later.map((f) => f.dim)).toEqual([true]);
        expect(flowsAt(later, 1_000 + PR_FRESH_MS + 60_000)).toBe(later);
        expect(flowsAt(later, 1_000 + PR_SHOWN_MS)).toEqual([]);
    });

    it("go when the pull request ends, and leave the others", () => {
        const flows = keep(
            [reviewFlow(review()), checksFlow(checks()), reviewFlow(review({ id: "d", pr: { repo_id: 9, number: 65 } }))],
            1_000
        );
        expect(withoutPr(flows, "9/64").map((f) => f.id)).toEqual(["review:d"]);
        expect(withoutPr(flows, "9/70")).toBe(flows);
    });

    it("are at most a dozen of each kind: the oldest go first", () => {
        const many = Array.from({ length: MAX_PR_LINES + 3 }, (_, i) =>
            reviewFlow(review({ id: `r${i}`, at_ms: 1_000 + i, pr: { repo_id: 9, number: 100 + i } }))
        );
        const flows = keep([checksFlow(checks()), ...many], 2_000);
        expect(flows.filter((f) => f.kind === "review")).toHaveLength(MAX_PR_LINES);
        expect(flows.some((f) => f.id === "review:r0")).toBe(false);
        expect(flows.some((f) => f.kind === "checks")).toBe(true);
    });
});

describe("the ends of a pull request's line", () => {
    const machine = (key: string, title: string): AtlasMachine => ({ key, title, badge: "LAN", platform: null, stale: false, seenAtMs: null });
    const agent = (name: string, m: string, remote: boolean): AtlasAgent => ({
        key: `${m}/${name}`,
        name,
        provider: null,
        state: "idle",
        line: null,
        machine: m,
        agentId: null,
        seat: null,
        contextTokens: null,
        currentTool: null,
        blockId: null,
        runtimeId: null,
        remote,
        stale: false,
    });
    const machines = new Map([machine("local", "desk"), machine("lab", "lab-1")].map((m) => [m.key, m] as const));

    it("are the service's node, and this instance's own agent of that name", () => {
        const resolve = flowEndResolver([agent("Ann", "local", false), agent("Ann", "lab", true)], machines);
        const flow = reviewFlow(review())!;
        expect(resolve(flow.from)).toBe(REVIEW_NODE);
        expect(resolve(flow.to)).toBe("local/Ann");
    });

    it("never an agent of another machine that has the same name", () => {
        const resolve = flowEndResolver([agent("Ann", "lab", true)], machines);
        expect(resolve(reviewFlow(review())!.to)).toBeNull();
    });
});

describe("prServices", () => {
    const time = (ms: number) => `t${ms}`;
    const bot = { lastNoticeAtMs: 5_000, paused: false };

    it("draws neither node until a notice has come", () => {
        expect(prServices([], { lastNoticeAtMs: null, paused: false }, time)).toEqual([]);
        expect(prServices([], bot, time).map((s) => s.key)).toEqual([REVIEW_NODE, CHECKS_NODE]);
        expect(prServices(keep([reviewFlow(review())], 1_000), { lastNoticeAtMs: null, paused: false }, time)).toHaveLength(2);
    });

    it("says when the last notice came, and what the checks on show are", () => {
        const [reviews, ci] = prServices(keep([reviewFlow(review()), checksFlow(checks())], 1_000), bot, time);
        expect(reviews).toMatchObject({ name: "Review bot", state: "ok", sub: "Last notice t5000" });
        expect(ci).toMatchObject({ name: "CI", state: "ok", sub: "1 passing" });
        expect(prServices([], bot, time)[1].sub).toBe("No results on show");
    });

    it("warns of a pull request that needs changes or fails its checks", () => {
        const flows = keep(
            [
                reviewFlow(review({ state: "changes_requested" })),
                reviewFlow(review({ id: "x", state: "changes_requested", pr: { repo_id: 9, number: 65 } })),
                checksFlow(checks({ state: "failing" })),
                checksFlow(checks({ id: "y", pr: { repo_id: 9, number: 65 } })),
            ],
            1_000
        );
        const [reviews, ci] = prServices(flows, bot, time);
        expect(reviews).toMatchObject({ state: "warn", sub: "2 need changes" });
        expect(ci).toMatchObject({ state: "warn", sub: "1 failing" });
        expect(prServices(flows.slice(0, 1), bot, time)[0].sub).toBe("1 needs changes");
    });

    it("says so when notices are not arriving", () => {
        const flows = keep([checksFlow(checks({ state: "failing" }))], 1_000);
        const services = prServices(flows, { lastNoticeAtMs: 5_000, paused: true }, time);
        expect(services.map((s) => `${s.state}: ${s.sub}`)).toEqual(["paused: Cloud messages paused", "paused: Cloud messages paused"]);
    });

    it("lists each pull request it has a line for, newest first, in words", () => {
        const flows = keep(
            [
                reviewFlow(review({ state: "changes_requested", by: "codex" })),
                reviewFlow(review({ id: "x", at_ms: 2_000, agent: "bo", pr: { repo_id: 9, number: 65 } })),
            ],
            2_000
        );
        expect(prServices(flows, bot, time)[0].lines).toEqual([
            { id: "review:x", pr: "#65", state: "approved", tone: "passed", agent: "bo", atMs: 2_000 },
            { id: "review:inj-1", pr: "#64 Codex", state: "changes requested", tone: "failed", agent: "ann", atMs: 1_000 },
        ]);
    });
});
