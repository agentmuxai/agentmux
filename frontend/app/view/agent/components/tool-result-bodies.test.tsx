// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Built-in result bodies that used to hide their content behind CompactResult's
 * chevron (SPEC_TOOL_PREVIEW_CONTENT_FIRST_2026_09_26.md §3.5–3.6): an Agent
 * report renders as markdown, and a Grep shows its matching lines without the
 * `Pattern:` line the row header already shows.
 */

import { cleanup, render, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import type { ToolNode } from "../types";
import { ToolOverlayLog } from "./ToolOverlayLog";

afterEach(() => cleanup());

const node = (over: Partial<ToolNode>): ToolNode => ({
    type: "tool",
    id: "t1",
    tool: "Other",
    params: {},
    status: "success",
    collapsed: false,
    summary: "x",
    ...over,
});

describe("Agent report body", () => {
    it("renders the report as markdown, not a `0: {2 keys}` one-liner", async () => {
        const { container } = render(() => (
            <ToolOverlayLog
                node={node({
                    tool: "Agent",
                    toolName: "Agent",
                    params: { description: "Audit the renderers" },
                    result: { content: "## Findings\n\n- one\n- two" } as any,
                })}
            />
        ));
        const report = container.querySelector(".agent-tool-agent-report");
        expect(report).not.toBeNull();
        await waitFor(() => expect(report!.textContent).toContain("Findings"));
        expect(container.querySelector(".agent-tool-compact-summary")).toBeNull();
        // The row header already shows the description (tool-header.ts).
        expect(container.textContent).not.toContain("Audit the renderers");
    });
});

describe("Agent report cap", () => {
    it("caps a very long report and says how much is hidden", async () => {
        const long = Array.from({ length: 1500 }, (_, i) => `line ${i}`).join("\n\n");
        const { container } = render(() => (
            <ToolOverlayLog node={node({ tool: "Agent", toolName: "Agent", result: { content: long } as any })} />
        ));
        expect(container.querySelector(".agent-output-hidden-marker")).not.toBeNull();
    });
});

describe("Grep reading order", () => {
    it("a long match list opens on its first lines", () => {
        const lines = Array.from({ length: 1200 }, (_, i) => `f${i}.ts:1:x`).join("\n");
        const { container } = render(() => (
            <ToolOverlayLog node={node({ tool: "Grep", toolName: "Grep", params: { pattern: "x" }, result: { content: lines } as any })} />
        ));
        expect(container.textContent).toContain("f0.ts:1:x");
    });
});

describe("Workflow body", () => {
    const wf = (params: Record<string, string>) =>
        node({ tool: "Workflow", toolName: "Workflow", params, result: { status: "done" } as any });

    it("shows the description only when the header shows a different title", () => {
        const both = render(() => <ToolOverlayLog node={wf({ title: "Review", description: "Review the diff" })} />);
        expect(both.container.textContent).toContain("Review the diff");
        cleanup();
        const descOnly = render(() => <ToolOverlayLog node={wf({ description: "Review the diff" })} />);
        expect(descOnly.container.textContent).not.toContain("Review the diff");
    });
});

describe("Grep body", () => {
    it("shows the matching lines directly and drops the Pattern line", () => {
        const { container } = render(() => (
            <ToolOverlayLog
                node={node({
                    tool: "Grep",
                    toolName: "Grep",
                    params: { pattern: "summary" },
                    result: { content: "a.ts:1:summary\nb.ts:2:summary" } as any,
                })}
            />
        ));
        expect(container.textContent).not.toContain("Pattern:");
        expect(container.querySelector(".agent-tool-compact-summary")).toBeNull();
        expect(container.textContent).toContain("b.ts:2:summary");
    });
});
