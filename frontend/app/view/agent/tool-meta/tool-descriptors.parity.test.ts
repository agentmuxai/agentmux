// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Parity table for the tool-descriptor migration
 * (SPEC_AGENT_PANE_TOOL_DESCRIPTORS_2026_09_26.md §5).
 *
 * Every row was captured from the pre-migration code (header parts, summary,
 * content-first, start-at-top, Activity Dock title, working-row argument) for
 * a real tool name. Rows marked CHANGED are the spec's §1.1 intended changes;
 * everything else must come out identical.
 */

import { describe, expect, it } from "vitest";
import { toolToActivity } from "../activity/tool-adapter";
import { toolHeaderParts } from "../components/tool-header";
import { ClaudeCodeStreamParser } from "../stream-parser";
import type { ToolNode } from "../types";
import { isContentFirstTool, startsAtTop, toolActivityArg } from "./tool-descriptors";

interface Row {
    name: string;
    params: Record<string, any>;
    icon: string;
    label: string | null;
    detail: string;
    summary: string;
    contentFirst: boolean;
    top: boolean;
    activity: string;
    arg: string | undefined;
}

// prettier-ignore
const ROWS: Row[] = [
    { name: "Read", params: { file_path: "src/a.ts" }, icon: "📖", label: "Read", detail: "src/a.ts", summary: "📖 Read src/a.ts", contentFirst: false, top: true, activity: "src/a.ts", arg: "src/a.ts" },
    { name: "Write", params: { file_path: "src/b.md", content: "x" }, icon: "📝", label: "Write", detail: "src/b.md", summary: "📝 Write src/b.md", contentFirst: false, top: true, activity: "src/b.md", arg: "src/b.md" },
    { name: "Edit", params: { file_path: "src/c.ts" }, icon: "✏️", label: "Edit", detail: "src/c.ts", summary: "✏️ Edit src/c.ts", contentFirst: false, top: true, activity: "src/c.ts", arg: "src/c.ts" },
    { name: "Bash", params: { command: "ls -la" }, icon: "🔧", label: "Bash", detail: "ls -la", summary: "🔧 Bash ls -la", contentFirst: false, top: false, activity: "ls -la", arg: "ls -la" },
    { name: "Grep", params: { pattern: "foo" }, icon: "🔍", label: "Grep", detail: "foo", summary: "🔍 Grep foo", contentFirst: false, top: false, activity: "foo", arg: "foo" },
    { name: "Glob", params: { pattern: "**/*.ts" }, icon: "📁", label: "Glob", detail: "**/*.ts", summary: "📁 Glob **/*.ts", contentFirst: false, top: false, activity: "**/*.ts", arg: "**/*.ts" },
    // CHANGED (§1.1): working-row argument, previously none.
    { name: "Agent", params: { description: "Audit", prompt: "p" }, icon: "🤖", label: "Agent", detail: "Audit", summary: "🤖 Agent Audit", contentFirst: false, top: false, activity: "Audit", arg: "Audit" },
    { name: "Task", params: { description: "d" }, icon: "🛠️", label: "Task", detail: "", summary: "🛠️ Task", contentFirst: false, top: false, activity: "Task", arg: undefined },
    // CHANGED (§1.1): working-row argument, previously none.
    { name: "Workflow", params: { title: "Review" }, icon: "🕸️", label: "Workflow", detail: "Review", summary: "🕸️ Workflow Review", contentFirst: false, top: false, activity: "Review", arg: "Review" },
    // CHANGED (§1.1): Activity Dock title, previously "WebSearch".
    { name: "WebSearch", params: { query: "solid docs" }, icon: "🌐", label: null, detail: "solid docs", summary: "🌐 WebSearch solid docs", contentFirst: true, top: true, activity: "solid docs", arg: "solid docs" },
    // CHANGED (§1.1): Activity Dock title, previously "web_search".
    { name: "web_search", params: { query: "q" }, icon: "🌐", label: null, detail: "q", summary: "🌐 web_search q", contentFirst: true, top: true, activity: "q", arg: "q" },
    // CHANGED (§1.1): Activity Dock title and working-row argument, previously "WebFetch" / none.
    { name: "WebFetch", params: { url: "https://example.com/a/b" }, icon: "🌐", label: null, detail: "example.com/a/b", summary: "🌐 WebFetch example.com/a/b", contentFirst: false, top: false, activity: "example.com/a/b", arg: "example.com/a/b" },
    { name: "ToolSearch", params: { query: "select:X" }, icon: "🛠️", label: "ToolSearch", detail: "", summary: "🛠️ ToolSearch", contentFirst: false, top: false, activity: "ToolSearch", arg: "select:X" },
    { name: "mcp__agentmux__WhoAmI", params: {}, icon: "🛠️", label: "agentmux · WhoAmI", detail: "", summary: "🛠️ mcp__agentmux__WhoAmI", contentFirst: false, top: false, activity: "mcp__agentmux__WhoAmI", arg: undefined },
    { name: "mcp__github__search_issues", params: { query: "bug" }, icon: "🛠️", label: "github · search_issues", detail: "", summary: "🛠️ mcp__github__search_issues", contentFirst: false, top: false, activity: "mcp__github__search_issues", arg: "bug" },
    { name: "TaskOutput", params: { task_id: "b1" }, icon: "🛠️", label: "TaskOutput", detail: "", summary: "🛠️ TaskOutput", contentFirst: false, top: false, activity: "TaskOutput", arg: undefined },
    // CHANGED (§1.1): icon, detail and start-at-top, previously 🛠️ / "" / false.
    { name: "read_file", params: { path: "src/d.ts" }, icon: "📖", label: "read_file", detail: "src/d.ts", summary: "📖 read_file src/d.ts", contentFirst: false, top: true, activity: "src/d.ts", arg: "src/d.ts" },
    // CHANGED (§1.1): icon, detail and start-at-top, previously 🛠️ / "" / false.
    { name: "str_replace_editor", params: { path: "src/e.ts" }, icon: "✏️", label: "str_replace_editor", detail: "src/e.ts", summary: "✏️ str_replace_editor src/e.ts", contentFirst: false, top: true, activity: "src/e.ts", arg: "src/e.ts" },
    // CHANGED (§1.1): header detail, previously "".
    { name: "computer", params: { command: "click" }, icon: "🛠️", label: "computer", detail: "click", summary: "🛠️ computer click", contentFirst: false, top: false, activity: "click", arg: "click" },
    { name: "AskUserQuestion", params: { questions: [] }, icon: "🛠️", label: "AskUserQuestion", detail: "", summary: "🛠️ AskUserQuestion", contentFirst: false, top: false, activity: "AskUserQuestion", arg: undefined },
];

describe("tool descriptors — parity with the pre-migration facts", () => {
    for (const r of ROWS) {
        it(r.name, () => {
            const p = new ClaudeCodeStreamParser();
            const call = p.parseStreamEvent({ type: "tool_call", tool: r.name, id: "x", params: r.params } as any) as ToolNode;
            const done: ToolNode = { ...call, status: "success" };
            // `range` is a Read's line range (tool-meta/read-range.ts): none of these rows set one.
            expect(toolHeaderParts(done)).toEqual({ icon: r.icon, label: r.label, detail: r.detail, range: null });
            expect(call.summary).toBe(r.summary);
            expect(isContentFirstTool(done)).toBe(r.contentFirst);
            expect(startsAtTop(done)).toBe(r.top);
            expect(toolToActivity(done).title).toBe(r.activity);
            expect(toolActivityArg(r.name, r.params)).toBe(r.arg);
        });
    }
});
