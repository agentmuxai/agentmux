// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Components inside a frozen segment must stay alive while the message keeps
 * streaming. An image in the frozen prefix resolves its path asynchronously; if
 * its component had been created under the insert effect that the next commit
 * re-runs, it would be disposed before the path resolved and never show.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const pending: Array<() => void> = [];
vi.mock("@/app/element/markdown-util", async (importOriginal) => {
    const orig = await importOriginal<typeof import("@/app/element/markdown-util")>();
    return {
        ...orig,
        resolveRemoteFile: (src: string) =>
            new Promise<string>((resolve) => pending.push(() => resolve(`resolved://${src}`))),
        resolveSrcSet: async () => "",
    };
});

vi.mock("@/util/clipboard", () => ({ writeText: async () => {} }));

import { Markdown } from "./markdown";

afterEach(() => cleanup());

const RESOLVE_OPTS: MarkdownResolveOpts = { connName: "", baseDir: "/" };
const LONG_FILLER = "Filler prose that pads the document past the split threshold.\n\n".repeat(14);

describe("Markdown frozen segment components", () => {
    it("an image in the frozen prefix still resolves after later commits", async () => {
        const head = `![pic](pic.png)\n\n${LONG_FILLER}`;
        const [text, setText] = createSignal(`${head}Streaming tail 0`);
        const { container } = render(() => (
            <Markdown text={text()} streaming={true} scrollable={false} resolveOpts={RESOLVE_OPTS} />
        ));
        await Promise.resolve();
        for (let i = 1; i <= 5; i++) setText(`${head}Streaming tail ${"x".repeat(i)}`);

        expect(pending.length).toBeGreaterThan(0);
        for (const r of pending.splice(0)) r();
        await new Promise((r) => setTimeout(r, 0));

        const img = container.querySelector("img");
        expect(img?.getAttribute("src")).toBe("resolved://pic.png");
    });

    it("a table's copy button still shows its feedback after the message finishes streaming", async () => {
        const head = `| a | b |\n|---|---|\n| 1 | 2 |\n\n${LONG_FILLER}`;
        const [view, setView] = createSignal({ text: `${head}Streaming tail 0`, streaming: true });
        const { container } = render(() => (
            <Markdown text={view().text} streaming={view().streaming} scrollable={false} />
        ));
        for (let i = 1; i <= 5; i++) setView({ text: `${head}Streaming tail ${"x".repeat(i)}`, streaming: true });
        setView({ text: `${head}Streaming tail done.`, streaming: false });

        const button = container.querySelector<HTMLButtonElement>(".table-block button")!;
        expect(button.textContent).toBe("CSV");
        button.click();
        await new Promise((r) => setTimeout(r, 0));
        expect(button.textContent).toContain("copied");
    });
});
