// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The replay driver places a jekt that arrived mid-block the same way the live
 * pane and history replay do: the text block stays whole and the jekt follows
 * it (SPEC_JEKT_IMMEDIATE_DELIVERY_2026_09_28.md §2.2; Codex P2 on #3977 —
 * the driver used to drop the held jekt).
 */

import { describe, expect, it } from "vitest";

import { replayInstant } from "./replay";
import type { Fixture } from "./types";

const jekt =
    "[JEKT:FROM=reagent TO=lark TIER=coord DELIVERY=wan TRUST=network-claimed MSGID=m1 PRIORITY=normal TS=1783386012]\n" +
    "From: reagent | To: lark | ts=1783386012\nPR reviewed\n[/JEKT]";

const fixture = (lines: object[]): Fixture =>
    ({
        header: {
            kind: "header",
            version: 1,
            agentmux_version: "test",
            schema_version: 8,
            recorded_at: "2026-09-28T00:00:00Z",
            provider: "claude",
            block_id: "b1",
            instance_name: "test",
            redactions: [],
        },
        events: lines.map((l, i) => ({ seq: i, t_ms: i, src: "stream-json" as const, line: JSON.stringify(l) })),
    }) as unknown as Fixture;

describe("replay driver — jekt arriving mid-block", () => {
    it("keeps the text block whole and puts the jekt after it, before the tool", () => {
        const result = replayInstant(
            fixture([
                { type: "text", content: "first half " },
                { type: "user_message", message: jekt },
                { type: "text", content: "second half" },
                { type: "tool_call", tool: "Bash", id: "tool-1", params: { command: "ls" } },
            ])
        );
        const nodes = result.docState.nodes;
        expect(nodes.map((n) => n.type)).toEqual(["markdown", "jekt_message", "tool"]);
        expect((nodes[0] as { content: string }).content).toBe("first half second half");
        expect(result.warnings.filter((w) => w.includes("produced no node"))).toEqual([]);
    });

    it("a fixture that ends mid-block still shows the held jekt", () => {
        const result = replayInstant(
            fixture([
                { type: "text", content: "still writing" },
                { type: "user_message", message: jekt },
            ])
        );
        expect(result.docState.nodes.map((n) => n.type)).toEqual(["markdown", "jekt_message"]);
    });
});
