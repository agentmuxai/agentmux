// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { TerminalOutput } from "./TerminalOutput";
import { MAX_TOOL_OUTPUT_LINES } from "./output-cap";

afterEach(() => cleanup());

describe("TerminalOutput", () => {
    it("renders one AnsiLine row per line", () => {
        const { container } = render(() => <TerminalOutput text={"a\nb\nc"} />);
        const root = container.querySelector(".agent-terminal-output")!;
        expect(root).not.toBeNull();
        // AnsiLine emits a <div> per line (no marker under the cap).
        expect(root.querySelectorAll(":scope > div:not(.agent-output-hidden-marker)").length).toBe(3);
        expect(root.textContent).toContain("a");
        expect(root.textContent).toContain("c");
    });

    it("colorizes ANSI SGR sequences via text-ansi-* classes", () => {
        const { container } = render(() => (
            <TerminalOutput text={"\x1b[31mred\x1b[0m plain"} />
        ));
        const colored = container.querySelector(".text-ansi-red");
        expect(colored).not.toBeNull();
        expect(colored!.textContent).toBe("red");
    });

    it("caps beyond MAX_TOOL_OUTPUT_LINES (tail) and shows a hidden marker", () => {
        const text = Array.from({ length: MAX_TOOL_OUTPUT_LINES + 5 }, (_, i) => `line${i}`).join("\n");
        const { container } = render(() => <TerminalOutput text={text} from="tail" />);
        const root = container.querySelector(".agent-terminal-output")!;
        // Exactly MAX line rows (the hidden-marker div is excluded).
        expect(root.querySelectorAll(":scope > div:not(.agent-output-hidden-marker)").length).toBe(MAX_TOOL_OUTPUT_LINES);
        const marker = container.querySelector(".agent-output-hidden-marker");
        expect(marker).not.toBeNull();
        expect(marker!.textContent).toContain("5");
        // tail-cap keeps the latest lines.
        expect(root.textContent).toContain(`line${MAX_TOOL_OUTPUT_LINES + 4}`);
        expect(root.textContent).not.toContain("line0\n");
    });

    it("renders no marker when under the cap", () => {
        const { container } = render(() => <TerminalOutput text={"short"} />);
        expect(container.querySelector(".agent-output-hidden-marker")).toBeNull();
    });
});
