// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Edit tool's diff preview must survive Shiki resolving, and must survive
 * the node being updated afterwards.
 *
 * User-reported regression: "I see the edited preview for a moment, but then it
 * disappears and is replaced by a single path to the file."
 */

import { render, cleanup, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";

// Shiki is a heavy async ESM import; stub it with a synchronous-resolving
// codeToHtml so the highlighted branch is reachable in a test.
vi.mock("shiki/bundle/web", () => ({
    codeToHtml: async (code: string) =>
        `<pre><code>${code
            .split("\n")
            .map((l) => `<span class="line">${l}</span>`)
            .join("")}</code></pre>`,
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

describe("DiffViewer — the preview survives the Shiki swap", () => {
    afterEach(cleanup);

    it("shows the diff body before Shiki resolves (plain fallback)", () => {
        const { container } = render(() => <DiffViewer params={params()} status="success" />);
        expect(visibleText(container)).toContain("const b = 3;");
    });

    /** The regression: after the highlighted branch mounts, the body must not
     *  be empty — leaving only the file-path header on screen. */
    it("still shows the diff body AFTER Shiki resolves", async () => {
        const { container } = render(() => <DiffViewer params={params()} status="success" />);
        await waitFor(() => {
            expect(container.querySelector(".agent-diff--highlighted")).toBeTruthy();
        });
        const body = container.querySelector(".agent-diff-highlighted-body");
        expect(body, "highlighted body element exists").toBeTruthy();
        expect(
            body!.innerHTML,
            "highlighted <pre> must be filled — an empty one leaves only the path header",
        ).not.toBe("");
        expect(visibleText(container)).toContain("const b = 3;");
    });

    /** A remount with a warm highlight cache: the html is resolved from the
     *  module-level `diffCache` synchronously, so it can be known BEFORE the
     *  <Show> has created the <pre> that receives it. An effect alone can run
     *  while `preEl` is still undefined and then never re-run, leaving an
     *  empty <pre> under a mounted `.agent-diff--highlighted` — the live
     *  failure (height 0, textContent ""). */
    it("fills the highlighted body on a remount that hits the highlight cache", async () => {
        // First mount warms diffCache for this (filePath, diff) key.
        const first = render(() => <DiffViewer params={params()} status="success" />);
        await waitFor(() => {
            expect(first.container.querySelector(".agent-diff--highlighted")).toBeTruthy();
        });
        cleanup();

        // Second mount: same key, so the html is available immediately.
        const { container } = render(() => <DiffViewer params={params()} status="success" />);
        await waitFor(() => {
            expect(container.querySelector(".agent-diff--highlighted")).toBeTruthy();
        });
        const body = container.querySelector(".agent-diff-highlighted-body");
        expect(body!.innerHTML, "a warm-cache remount must still fill the <pre>").not.toBe("");
        expect(visibleText(container)).toContain("const b = 3;");
    });

    /** The exact user-visible sequence: the node updates once more after the
     *  highlight (the reducer replaces `params` wholesale on every node
     *  update), which re-runs the highlight effect: null -> plain -> cached.
     *  The body must come back, not vanish. */
    it("still shows the diff body after the node updates post-highlight", async () => {
        const [p, setP] = createSignal(params());
        const { container } = render(() => <DiffViewer params={p()} status="success" />);

        await waitFor(() => {
            expect(container.querySelector(".agent-diff--highlighted")).toBeTruthy();
        });

        // Same content, new object identity — what mergeReplacement produces.
        setP(params());

        await waitFor(() => {
            expect(container.querySelector(".agent-diff--highlighted")).toBeTruthy();
        });
        expect(
            visibleText(container),
            "the diff must not collapse to just the file path",
        ).toContain("const b = 3;");
    });
});
