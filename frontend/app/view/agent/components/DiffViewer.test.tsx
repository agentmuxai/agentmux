// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Edit tool's diff preview must survive Shiki resolving, and must survive
 * the node being updated afterwards.
 *
 * User-reported regression: "I see the edited preview for a moment, but then it
 * disappears and is replaced by a single path to the file." The diff is now one
 * block per line drawn from the preview text stage, highlighted with tokens per
 * line (REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md §5.2), so there is no
 * second element for Shiki's HTML to land in; these keep the guarantee.
 */

import { render, cleanup, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";

// Shiki is a heavy async ESM import; stub it with tokens per line (one red
// token each) and record what it was asked to tokenize.
const tokenized = vi.hoisted(() => [] as string[]);
vi.mock("./shiki-highlighter", () => ({
    codeToTokens: async (code: string) => {
        tokenized.push(code);
        return { tokens: code.split("\n").map((l) => [{ content: l, color: "#ff0000" }]) };
    },
}));

import { DiffViewer } from "./DiffViewer";

const params = (over: Record<string, unknown> = {}) => ({
    file_path: "/repo/src/thing.ts",
    old_string: "const a = 1;\nconst b = 2;",
    new_string: "const a = 1;\nconst b = 3;",
    ...over,
}) as any;

/** Everything the user can actually read in the rendered preview. */
const visibleText = (c: HTMLElement) => c.textContent ?? "";
const highlighted = (c: HTMLElement) => c.querySelector('.agent-preview-text span[style*="color"]');

describe("DiffViewer — the preview survives the Shiki swap", () => {
    afterEach(cleanup);

    it("shows the diff body before Shiki resolves (plain text)", () => {
        const { container } = render(() => <DiffViewer params={params()} status="success" />);
        expect(visibleText(container)).toContain("const b = 3;");
    });

    it("still shows the diff body AFTER Shiki resolves", async () => {
        const { container } = render(() => <DiffViewer params={params()} status="success" />);
        await waitFor(() => expect(highlighted(container)).toBeTruthy());
        expect(visibleText(container)).toContain("const b = 3;");
        expect(container.querySelector(".agent-diff-header")!.textContent).toBe("/repo/src/thing.ts");
    });

    it("still shows it on a remount that hits the highlight cache", async () => {
        const first = render(() => <DiffViewer params={params()} status="success" />);
        await waitFor(() => expect(highlighted(first.container)).toBeTruthy());
        cleanup();
        const { container } = render(() => <DiffViewer params={params()} status="success" />);
        await waitFor(() => expect(highlighted(container)).toBeTruthy());
        expect(visibleText(container)).toContain("const b = 3;");
    });

    it("still shows the diff body after the node updates post-highlight", async () => {
        const [p, setP] = createSignal(params());
        const { container } = render(() => <DiffViewer params={p()} status="success" />);
        await waitFor(() => expect(highlighted(container)).toBeTruthy());
        // Same content, new object identity — what mergeReplacement produces.
        setP(params());
        await waitFor(() => expect(highlighted(container)).toBeTruthy());
        expect(visibleText(container), "the diff must not collapse to just the file path").toContain("const b = 3;");
    });
});

describe("DiffViewer — one block per line", () => {
    afterEach(cleanup);

    it("marks each line by kind, with the marker outside the text", () => {
        const { container } = render(() => <DiffViewer params={params()} status="success" />);
        const lines = [...container.querySelectorAll(".agent-preview-line")];
        expect(lines.map((l) => [...l.classList].find((c) => c.startsWith("agent-preview-line--")))).toEqual([
            "agent-preview-line--hunk",
            "agent-preview-line--ctx",
            "agent-preview-line--del",
            "agent-preview-line--add",
        ]);
        expect(lines[3].querySelector(".agent-preview-marker")!.textContent).toBe("+");
        expect(lines[3].querySelector(".agent-preview-text")!.textContent).toBe("const b = 3;");
    });

    it("tokenizes the old and new sides separately, so grammar state can't cross", async () => {
        tokenized.length = 0;
        // Content no earlier test used, so neither side is in the highlight cache.
        const { container } = render(() => (
            <DiffViewer params={params({ old_string: "let s = `a\nb`;", new_string: "let s = `a\nc`;" })} status="success" />
        ));
        await waitFor(() => expect(highlighted(container)).toBeTruthy());
        expect(tokenized.sort()).toEqual(["let s = `a\nb`;", "let s = `a\nc`;"]);
    });

    it("says there is no diff for an edit that did not apply", () => {
        const { container } = render(() => <DiffViewer params={params()} status="failed" />);
        expect(container.querySelector(".agent-diff-empty")).toBeTruthy();
    });
});
