// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    FALLBACK_TOOLTIP,
    heuristicTitle,
    lastPromptToStore,
    LAST_PROMPT_MAX_CHARS,
    META_AWAITING_USER,
    META_LAST_PROMPT,
    META_RESTORED,
    resolveSwarmLine,
    RESTORED_TOOLTIP,
    STATUS_NO_ACTIVITY,
    STATUS_SUMMARIZING,
    STATUS_THINKING,
    STATUS_WAITING,
    STATUS_WORKING,
    swarmLineTooltip,
    type SwarmLineInput,
} from "./swarm-line";

const base: SwarmLineInput = { meta: {}, status: "idle", currentTool: null, contextTokens: null };
const line = (over: Partial<SwarmLineInput>) => resolveSwarmLine({ ...base, ...over });

describe("heuristicTitle", () => {
    it.each([
        ["Please fix the login redirect on the settings page", "Fix the login redirect on the settings page"],
        ["can you please review PR 4230", "Review PR 4230"],
        ["ok so could you look at why the swarm row says none yet", "Look at why the swarm row says none…"],
        ["Hey, let's add a retry to the mDNS watchdog", "Add a retry to the mDNS watchdog"],
        ["I need you to write the spec for UDP discovery", "Write the spec for UDP discovery"],
        ["Run the full vitest suite and tell me what fails. Then fix it.", "Run the full vitest suite and tell me…"],
        ["merge on approval", "Merge on approval"],
        ["Refactor the pane tab registry\nand move the tests along with it", "Refactor the pane tab registry"],
    ])("turns %j into %j", (message, expected) => {
        expect(heuristicTitle(message)).toBe(expected);
    });

    it("cuts a long request to eight words with an ellipsis", () => {
        expect(heuristicTitle("Investigate why the agent pane header flickers when the window is resized quickly")).toBe(
            "Investigate why the agent pane header flickers when…"
        );
    });

    it.each([
        "u there",
        "U there?",
        "continue",
        "Continue.",
        "ok",
        "yes",
        "thanks!",
        "hi",
        "how's it going?",
        "use your recommendations",
        "",
        "   ",
        "please",
        "ok so",
        "can you",
    ])("finds no goal in the nudge %j", (message) => {
        expect(heuristicTitle(message)).toBeNull();
    });

    it("never turns an agent-to-agent or broadcast message into a goal", () => {
        expect(heuristicTitle("[JEKT:FROM=github-consumer TO=agentx TIER=coord] PR merged")).toBeNull();
        expect(heuristicTitle("[BROADCAST:FROM=user VIA=swarm TO=agentx] do your part")).toBeNull();
    });

    it("never turns text about the absence of a title into one", () => {
        expect(heuristicTitle("none yet")).toBeNull();
        expect(heuristicTitle("no title yet")).toBeNull();
    });

    it("leaves null and undefined alone", () => {
        expect(heuristicTitle(null)).toBeNull();
        expect(heuristicTitle(undefined)).toBeNull();
    });
});

describe("lastPromptToStore", () => {
    it("keeps a message with a goal, capped", () => {
        expect(lastPromptToStore("  Fix the login redirect  ")).toBe("Fix the login redirect");
        expect(lastPromptToStore("fix the thing ".repeat(100))!.length).toBe(LAST_PROMPT_MAX_CHARS);
    });

    it("keeps nothing for a nudge, so it never overwrites a real goal", () => {
        expect(lastPromptToStore("u there")).toBeNull();
        expect(lastPromptToStore("[JEKT:FROM=x] hello there friend")).toBeNull();
    });
});

describe("resolveSwarmLine", () => {
    it("prefers the generated title over everything", () => {
        const meta = {
            "term:ambient_summary": "Fix the login race",
            [META_RESTORED]: "Old goal",
            [META_LAST_PROMPT]: "Please review something else entirely",
            [META_AWAITING_USER]: true,
        };
        expect(line({ meta, status: "running", currentTool: "Bash" })).toEqual({
            text: "Fix the login race",
            source: "generated",
        });
    });

    it("falls back to the CLI's own title as generated, as before", () => {
        expect(line({ meta: { "term:osc_title": "Refactor the router" } })).toEqual({
            text: "Refactor the router",
            source: "generated",
        });
    });

    it("never shows a placeholder as a title, whichever rung it is on", () => {
        const meta = {
            "term:ambient_summary": "(none yet)",
            [META_RESTORED]: "No goal established yet",
            [META_LAST_PROMPT]: "u there",
        };
        expect(line({ meta })).toEqual({ text: STATUS_NO_ACTIVITY, source: "status" });
    });

    it("says it is waiting for you ahead of an old goal, but not ahead of a generated title", () => {
        const waiting = { [META_AWAITING_USER]: true, [META_RESTORED]: "Old goal", [META_LAST_PROMPT]: "Fix the login race" };
        expect(line({ meta: waiting, status: "running" })).toEqual({ text: STATUS_WAITING, source: "status" });
        expect(line({ meta: { ...waiting, "term:ambient_summary": "Fix the login race" } }).source).toBe("generated");
        // And it is only the true flag: a cleared one falls through the ladder.
        expect(line({ meta: { ...waiting, [META_AWAITING_USER]: false } }).source).toBe("restored");
    });

    it("uses the restored title next, then the heuristic one", () => {
        const both = { [META_RESTORED]: "Set up CI for the docs site", [META_LAST_PROMPT]: "Please fix the login redirect" };
        expect(line({ meta: both })).toEqual({ text: "Set up CI for the docs site", source: "restored" });
        expect(line({ meta: { [META_LAST_PROMPT]: "Please fix the login redirect" } })).toEqual({
            text: "Fix the login redirect",
            source: "heuristic",
        });
    });

    it("describes what a running agent is doing when nothing else is known", () => {
        expect(line({ status: "running", currentTool: "Bash" })).toEqual({ text: STATUS_WORKING, source: "status" });
        expect(line({ status: "running" })).toEqual({ text: STATUS_THINKING, source: "status" });
    });

    it("tells an idle agent with history from one that has done nothing", () => {
        expect(line({ contextTokens: 52_000 })).toEqual({ text: STATUS_SUMMARIZING, source: "status" });
        expect(line({ contextTokens: 0 })).toEqual({ text: STATUS_NO_ACTIVITY, source: "status" });
        expect(line({ contextTokens: null })).toEqual({ text: STATUS_NO_ACTIVITY, source: "status" });
    });

    it("is never empty, whatever the meta holds", () => {
        const junk = [undefined, {}, { "term:ambient_summary": "" }, { "term:ambient_summary": 5 }, { [META_RESTORED]: null }, { [META_LAST_PROMPT]: 7 }];
        for (const meta of junk) {
            for (const status of ["idle", "running"] as const) {
                expect(line({ meta: meta as any, status }).text.length).toBeGreaterThan(0);
            }
        }
    });
});

describe("swarmLineTooltip", () => {
    it("labels every fallback and leaves the agent's own title alone", () => {
        expect(swarmLineTooltip({ text: "x", source: "generated" })).toBeUndefined();
        expect(swarmLineTooltip({ text: "x", source: "restored" })).toBe(RESTORED_TOOLTIP);
        expect(swarmLineTooltip({ text: "x", source: "heuristic" })).toBe(FALLBACK_TOOLTIP);
        expect(swarmLineTooltip({ text: "x", source: "status" })).toBe(FALLBACK_TOOLTIP);
    });
});
