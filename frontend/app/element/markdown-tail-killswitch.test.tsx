// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `markdown:streamtailinplace` = false restores the previous behaviour: the
 * open tail is rebuilt on every commit, never patched in place. Kept in its own
 * file because it mocks the settings store for the whole module.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({
    openLink: () => {},
    getSettingsKeyAtom: (key: string) => () => (key === "markdown:streamtailinplace" ? false : undefined),
}));

import { Markdown, __markdownRenderStats, __resetMarkdownRenderStats } from "./markdown";

afterEach(() => cleanup());

const LONG_FILLER = "Filler prose that pads the document past the split threshold.\n\n".repeat(14);

describe("markdown:streamtailinplace = false (kill switch)", () => {
    it("rebuilds the open tail on every commit instead of patching it", () => {
        __resetMarkdownRenderStats();
        const lines = Array.from({ length: 10 }, (_, i) => `const line${i} = ${i};`);
        const doc = (n: number) => `${LONG_FILLER}\`\`\`ts\n${lines.slice(0, n).join("\n")}\n`;
        const [view, setView] = createSignal({ text: doc(1), streaming: true });
        const { container } = render(() => <Markdown text={view().text} streaming={view().streaming} scrollable={false} />);
        const first = container.querySelector("pre.codeblock");

        for (let n = 2; n <= 10; n++) setView({ text: doc(n), streaming: true });

        expect(__markdownRenderStats.tailInPlaceUpdates).toBe(0);
        expect(__markdownRenderStats.tailRebuilds).toBeGreaterThanOrEqual(10);
        expect(container.querySelector("pre.codeblock")).not.toBe(first);
        expect(container.querySelector("pre.codeblock code")!.textContent).toContain("const line9 = 9;");
    });
});
