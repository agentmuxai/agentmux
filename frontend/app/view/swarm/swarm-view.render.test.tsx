// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Component-level coverage for DispatchActivityFeedEntry — the Swarm view's
 * per-dispatch activity feed row. Two fixes covered here:
 *
 *   1. The `showAgentTag` gate: a solo Agent Tool dispatch's feed only ever
 *      contains its own agent's events, so tagging every line with the same
 *      unchanging 7-char hex id is pure noise. Only a genuine multi-member
 *      Workflow feed needs the tag. (Reported 2026-08-07 as "hex codes on
 *      nearly every line".)
 *   2. ANSI-aware rendering: captured shell output can carry SGR escape
 *      sequences with zero backend sanitization — these should render
 *      colorized (matching the Agent pane's own tool-result view), not as
 *      literal escape text.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { DispatchActivityFeedEntry } from "./swarm-view";
import type { DispatchActivityEntry } from "./swarm-model";

afterEach(() => cleanup());

function textEntry(agentId: string, content: string): DispatchActivityEntry {
    return {
        agentId,
        event: { agent_id: agentId, timestamp: 0, event_type: { type: "text", content } },
    };
}

function toolResultEntry(agentId: string, preview: string, isError = false): DispatchActivityEntry {
    return {
        agentId,
        event: { agent_id: agentId, timestamp: 0, event_type: { type: "tool_result", is_error: isError, preview } },
    };
}

function progressEntry(agentId: string, output: string): DispatchActivityEntry {
    return {
        agentId,
        event: { agent_id: agentId, timestamp: 0, event_type: { type: "progress", output } },
    };
}

describe("DispatchActivityFeedEntry — agent tag visibility", () => {
    it("renders the agent tag when showAgentTag is true (multi-member workflow feed)", () => {
        const { container } = render(() => (
            <DispatchActivityFeedEntry entry={textEntry("abcdef1234567", "hello")} showAgentTag={true} />
        ));
        const tag = container.querySelector(".swarm-dispatch-feed-tag");
        expect(tag).not.toBeNull();
        expect(tag!.textContent).toBe("abcdef1");
    });

    it("omits the agent tag when showAgentTag is false (solo Agent Tool feed)", () => {
        const { container } = render(() => (
            <DispatchActivityFeedEntry entry={textEntry("abcdef1234567", "hello")} showAgentTag={false} />
        ));
        expect(container.querySelector(".swarm-dispatch-feed-tag")).toBeNull();
    });

    it("gates the tag consistently across every entry kind (text, tool_result, progress)", () => {
        for (const entry of [
            textEntry("a1", "hi"),
            toolResultEntry("a1", "ok"),
            progressEntry("a1", "working"),
        ]) {
            const { container, unmount } = render(() => (
                <DispatchActivityFeedEntry entry={entry} showAgentTag={false} />
            ));
            expect(container.querySelector(".swarm-dispatch-feed-tag")).toBeNull();
            unmount();
        }
    });
});

describe("DispatchActivityFeedEntry — ANSI-aware rendering", () => {
    it("colorizes ANSI SGR sequences in a text entry via text-ansi-* classes, instead of literal escape text", () => {
        const { container } = render(() => (
            <DispatchActivityFeedEntry
                entry={textEntry("a1", "\x1b[31mred\x1b[0m plain")}
                showAgentTag={false}
            />
        ));
        const colored = container.querySelector(".text-ansi-red");
        expect(colored).not.toBeNull();
        expect(colored!.textContent).toBe("red");
        expect(container.textContent).toContain("plain");
    });

    it("colorizes ANSI in an expanded tool_result preview", async () => {
        const { container, findByText } = render(() => (
            <DispatchActivityFeedEntry
                entry={toolResultEntry("a1", "\x1b[32mok\x1b[0m", false)}
                showAgentTag={false}
            />
        ));
        // tool_result body only renders once expanded.
        const header = await findByText("Result");
        header.click();
        expect(container.querySelector(".text-ansi-green")).not.toBeNull();
    });

    it("splits a multi-line body into one row per line", () => {
        const { container } = render(() => (
            <DispatchActivityFeedEntry entry={textEntry("a1", "line1\nline2\nline3")} showAgentTag={false} />
        ));
        const body = container.querySelector(".swarm-subagent-detail-text")!;
        expect(body.children.length).toBe(3);
        expect(body.textContent).toBe("line1line2line3");
    });

    it("does not route progress output through AnsiText (inline layout, not a captured command body)", () => {
        const { container } = render(() => (
            <DispatchActivityFeedEntry entry={progressEntry("a1", "\x1b[31mworking\x1b[0m")} showAgentTag={false} />
        ));
        // Still renders literally (unchanged behavior) — asserts the
        // deliberate scoping decision, not a bug.
        expect(container.querySelector(".swarm-subagent-detail-progress span")!.textContent).toContain("working");
        expect(container.querySelector(".text-ansi-red")).toBeNull();
    });
});
