// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Parity for moving the result pills and CompactResult's per-tool facts into
 * the descriptor table (SPEC_AGENT_PANE_TOOL_DESCRIPTORS_2026_09_26.md PR 2).
 * Every expectation was captured from the pre-migration ToolBlock /
 * CompactResult; nothing here is meant to change.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { CompactResult } from "../components/CompactResult";
import { ToolBlock } from "../components/ToolBlock";
import type { ToolNode } from "../types";

afterEach(() => cleanup());

const node = (tool: ToolNode["tool"], toolName: string, result: unknown, status: ToolNode["status"] = "success"): ToolNode => ({
    type: "tool",
    id: "p",
    tool,
    toolName,
    params: {},
    status,
    collapsed: true,
    summary: "x",
    result: result as any,
});

const pillOf = (n: ToolNode): string | null => {
    const { container, unmount } = render(() => <ToolBlock node={n} pinned={false} onTogglePin={() => {}} />);
    const el = container.querySelector(".agent-tool-result-pill");
    const out = el ? `${el.textContent} ${[...el.classList].find((c) => c.startsWith("pill-"))}` : null;
    unmount();
    return out;
};

const WEB = 'Web search results for query: "q"\n\nLinks: [{"title":"A","url":"https://a.com"},{"title":"B","url":"https://b.com"}]\n\nSummary.';

// prettier-ignore
const PILLS: Array<[string, ToolNode, string | null]> = [
    ["Bash <exited 0>", node("Bash", "Bash", { stdout: "<exited 0 in 1.2s>\nok" }), "exit 0 pill-exit-ok"],
    ["Bash exitCode 2", node("Bash", "Bash", { exitCode: 2, stdout: "" }), "exit 2 pill-exit-err"],
    ["Bash no code", node("Bash", "Bash", { stdout: "hi" }), null],
    ["Codex bash alias", node("Bash", "bash", { exitCode: 0 }), "exit 0 pill-exit-ok"],
    ["Bash running", node("Bash", "Bash", { exitCode: 0 }, "running"), null],
    ["Glob files", node("Glob", "Glob", { files: ["a", "b"] }), "2 files pill-files"],
    ["Glob one file", node("Glob", "Glob", { files: ["a"] }), "1 file pill-files"],
    ["Glob text", node("Glob", "Glob", { content: "a\nb" }), null],
    ["Grep matches", node("Grep", "Grep", { matches: [1] }), "1 match pill-matches"],
    ["Grep files mode", node("Grep", "Grep", { content: "Found 3 files\na\nb\nc" }), "3 files pill-files"],
    ["Grep empty", node("Grep", "Grep", { content: "No matches found" }), "0 matches pill-matches"],
    ["Write bytes", node("Write", "Write", { bytesWritten: 10 }), "10b pill-written"],
    ["Write no bytes", node("Write", "Write", { ok: true }), "written pill-written"],
    ["Edit lines", node("Edit", "Edit", { linesChanged: 1 }), "1 line pill-edited"],
    ["Edit no count", node("Edit", "Edit", { ok: true }), "edited pill-edited"],
    ["WebSearch", node("Other", "WebSearch", { content: WEB }), "2 sources pill-sources"],
    ["WebSearch unparsed", node("Other", "WebSearch", { content: "plain" }), null],
    ["Read", node("Read", "Read", { content: "x" }), null],
    ["MCP", node("Other", "mcp__agentmux__WhoAmI", { content: "x" }), null],
    ["Agent, no dispatch match", node("Agent", "Agent", { content: "report" }), "done pill-agent"],
    ["Agent failed", node("Agent", "Agent", { content: "x" }, "failed"), null],
];

describe("result pills — parity", () => {
    for (const [name, n, want] of PILLS) it(name, () => expect(pillOf(n)).toBe(want));
});

const summaryOf = (tool: string, result: unknown): { text: string | null; expanded: boolean } => {
    const { container, unmount } = render(() => <CompactResult tool={tool} params={{}} result={result} />);
    const text = container.querySelector(".agent-tool-compact-text")?.textContent ?? null;
    const expanded = container.querySelector(".agent-tool-glob-files, .agent-tool-compact-json") !== null;
    unmount();
    return { text, expanded };
};

// prettier-ignore
const SUMMARIES: Array<[string, string, unknown, string, boolean]> = [
    ["Grep matches", "Grep", { matches: [1, 2] }, "2 matches found", false],
    ["Glob files, starts expanded", "Glob", { files: ["src/x/a.ts", "d.ts"] }, ".../x/a.ts, d.ts", true],
    ["Glob many files", "Glob", { files: ["a", "b", "c", "d", "e"] }, "a, b, c (+2 more)", true],
    ["Task status", "Task", { status: "done", id: 1 }, "Status: done", false],
    ["Workflow status", "Workflow", { status: "ok", id: 1 }, "Status: ok", false],
    ["generic, few keys", "Other", { a: 1, b: "two" }, 'a: 1, b: "two"', false],
    ["generic, many keys", "Other", { a: 1, b: 2, c: 3, d: 4 }, "{a, b, c +1 more}", false],
];

describe("CompactResult structured summaries — parity", () => {
    for (const [name, tool, result, text, expanded] of SUMMARIES) {
        it(name, () => expect(summaryOf(tool, result)).toEqual({ text, expanded }));
    }
});
