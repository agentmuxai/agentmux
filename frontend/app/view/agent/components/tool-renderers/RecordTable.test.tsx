// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { RecordTable } from "./RecordTable";
import { resolveToolRenderer } from "./registry";
import type { ToolNode } from "../../types";

afterEach(() => cleanup());

const node = (result: unknown, over: Partial<ToolNode> = {}): ToolNode => ({
    type: "tool",
    id: "t1",
    tool: "Other",
    params: {},
    status: "success",
    collapsed: true,
    summary: "x",
    result: result as any,
    ...over,
});

describe("RecordTable", () => {
    it("renders a table with a header per column and a row per record", () => {
        const { container } = render(() => (
            <RecordTable node={node([{ name: "a", count: 1 }, { name: "b", count: 2 }])} />
        ));
        const table = container.querySelector(".agent-tool-record-table table");
        expect(table).not.toBeNull();
        expect(container.querySelectorAll("thead th").length).toBe(2);
        expect(container.querySelectorAll("tbody tr").length).toBe(2);
        expect(container.textContent).toContain("name");
        expect(container.textContent).toContain("a");
    });

    it("falls back to JSON (CompactResult) for a non-record result", () => {
        const { container } = render(() => <RecordTable node={node({ status: "done" })} />);
        expect(container.querySelector(".agent-tool-record-table")).toBeNull();
        expect(container.querySelector(".agent-tool-compact-result")).not.toBeNull();
    });

    it("is registered by shape for an unknown tool's record list", () => {
        // Importing this module registered shape:record-table at priority -1
        // (above the JSON catch-all). An unknown tool with a record list routes
        // here rather than the JSON default.
        const r = resolveToolRenderer(node([{ a: 1 }], { tool: "Other" }));
        expect(r).not.toBeNull();
        const { container } = render(() => r!(node([{ a: 1 }], { tool: "Other" })));
        expect(container.querySelector(".agent-tool-record-table")).not.toBeNull();
    });
});
