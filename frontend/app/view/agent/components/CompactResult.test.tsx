// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CompactResult — expanded-body rendering. Asserts that a result carrying a
 * terminal string body renders as a TerminalOutput terminal panel, while a
 * purely structured result still falls back to the JSON <pre>.
 * SPEC_TOOL_OUTPUT_TEE_AND_TERMINAL_RENDER_2026_06_17.md §4.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { CompactResult } from "./CompactResult";

afterEach(() => cleanup());

function expand(container: HTMLElement): void {
    const summary = container.querySelector(".agent-tool-compact-summary.clickable") as HTMLElement | null;
    if (summary) fireEvent.click(summary);
}

describe("CompactResult — terminal vs JSON body", () => {
    it("renders a terminal panel (not JSON) when the result has a string body", () => {
        const { container } = render(() => (
            <CompactResult tool="Task" params={{}} result={{ content: "line1\nline2" }} />
        ));
        expand(container);
        expect(container.querySelector(".agent-terminal-output")).not.toBeNull();
        expect(container.querySelector(".agent-tool-compact-json")).toBeNull();
        expect(container.textContent).toContain("line1");
        expect(container.textContent).toContain("line2");
    });

    it("renders stdout/stderr as a terminal for command-shaped results", () => {
        const { container } = render(() => (
            <CompactResult tool="TaskOutput" params={{}} result={{ stdout: "hello\nworld" }} />
        ));
        expand(container);
        expect(container.querySelector(".agent-terminal-output")).not.toBeNull();
        expect(container.querySelector(".agent-tool-compact-json")).toBeNull();
    });

    it("falls back to JSON for a purely structured result", () => {
        const { container } = render(() => (
            <CompactResult tool="Task" params={{}} result={{ status: "done", count: 3, items: [1, 2, 3] }} />
        ));
        expand(container);
        expect(container.querySelector(".agent-terminal-output")).toBeNull();
        expect(container.querySelector(".agent-tool-compact-json")).not.toBeNull();
    });
});
