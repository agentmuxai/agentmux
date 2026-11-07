// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A reply that streams must not show a long raw form of something and then replace it with a short one:
 * the rows below it, and a pane following the bottom, would jump back. Each reply is fed one character at
 * a time, and the visible text may not drop by more than a few characters between two commits.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({
    openLink: () => {},
    getSettingsKeyAtom: () => () => undefined,
}));

import { MarkdownBlock } from "../view/agent/components/MarkdownBlock";
import type { MarkdownNode } from "../agent-model/types";
import { Markdown } from "./markdown";

afterEach(() => cleanup());

/** What a marker like `[` or a half-typed word may cost: a character or two, never a whole destination. */
const SLACK_CHARS = 3;

const REPLIES: Record<string, string> = {
    "a link": "See [the review notes](https://github.com/agentmuxai/agentmux/pull/6742#pullrequestreview-5811596546) for details.",
    "an image": "Here: ![the new toolbar after the fix](C:/work/shots/toolbar-after-the-fix.png) done.",
    "a link to a local file": "Open [the spec](C:/work/project/docs/specs/SPEC_X.md) now.",
    "a task link": "Task [#28724](agentmux://task/28724) is yours.",
    "two links in a line": "[one](https://example.com/a/long/path/one) and [two](https://example.com/a/long/path/two) end",
    "a link in a list item": "- first\n- see [the doc](https://example.com/a/long/path/to/the/doc) then\n- last",
    "a link with a title": 'Read [it](https://example.com/a/long/path "the title of the page") now.',
    "a colour span": 'Result: <span class="am-ok">All 387 tests pass</span> and more.',
    "a badge": 'Found <span class="am-badge am-error">P1</span> the loop never exits.',
    "a callout": "Intro.\n\n> [!WARNING]\n> Closing this removes the record.\n\nAfter.",
    "inline code": "Run `npx tsc --noEmit -p tsconfig.citypecheck.json` first.",
    "a bare URL": "Visit https://github.com/agentmuxai/agentmux/pull/6742 today.",
    "a table": "Intro\n\n| a | b |\n|---|---|\n| one | two |\n| three | four |\n\nAfter.",
    "a code fence": "Code:\n\n```ts\nconst a = 1;\nconst b = 2;\n```\n\nAfter.",
};

function sweep(text: string): { drops: string[]; settled: string } {
    const [view, setView] = createSignal({ text: "", streaming: true });
    const { container } = render(() => <Markdown text={view().text} streaming={view().streaming} scrollable={false} />);
    let prev = 0;
    const drops: string[] = [];
    for (let i = 1; i <= text.length; i++) {
        setView({ text: text.slice(0, i), streaming: true });
        const len = (container.textContent ?? "").length;
        if (len < prev - SLACK_CHARS) drops.push(`after ${JSON.stringify(text.slice(0, i).slice(-14))}: ${prev} -> ${len}`);
        prev = len;
    }
    setView({ text, streaming: false });
    return { drops, settled: container.textContent ?? "" };
}

describe("a streaming reply never shows a long raw form that then collapses", () => {
    for (const [name, text] of Object.entries(REPLIES)) {
        it(`${name}`, () => {
            expect(sweep(text).drops).toEqual([]);
        });
    }

    it("shows a link's label from the first character of its destination", () => {
        const [view] = createSignal({ text: "See [the notes](https://example.com/very/long/dest", streaming: true });
        const { container } = render(() => <Markdown text={view().text} streaming={view().streaming} scrollable={false} />);
        expect(container.textContent).toContain("See the notes");
        expect(container.textContent).not.toContain("https://example.com");
        expect(container.querySelector("a")?.textContent).toBe("the notes");
    });

    it("still shows code that looks like a link while streaming, in a fence and in an inline code span", () => {
        const fenced = render(() => <Markdown text={"```js\nconst v = pick[i](args"} streaming scrollable={false} />);
        expect(fenced.container.textContent).toContain("pick[i](args");
        cleanup();
        const inline = render(() => <Markdown text={"Call `pick[i](args"} streaming scrollable={false} />);
        expect(inline.container.textContent).toContain("pick[i](args");
    });

    it("settles an unfinished link in a message that has stopped, as a pause mid-stream renders settled too", () => {
        const { settled } = sweep("An unfinished [link](https://example.com/x");
        expect(settled).toContain("An unfinished link");
        expect(settled).not.toContain("https://example.com");
    });

    it("leaves the rest of a settled message exactly as written", () => {
        const { settled } = sweep("Plain text with `code`, a [link](https://example.com/x) and <b>html</b>.");
        expect(settled).toContain("Plain text with code, a link and html.");
    });
});

// MarkdownBlock cannot know when a stream has ended: after STREAM_RENDER_MS (90 ms) without an update it renders
// the message settled, and real streams pause longer than that all the time. That settled render must not show
// the raw address either, or the link collapses on the next chunk.
describe("a pause in the stream (a settled render with more text to come)", () => {
    const PAUSE_MS = 120;

    it.each([
        ["a link", REPLIES["a link"]],
        ["an image", REPLIES["an image"]],
        ["a link to a local file", REPLIES["a link to a local file"]],
        ["a task link", REPLIES["a task link"]],
    ])("%s never shows its raw address between chunks", (_name, text) => {
        vi.useFakeTimers();
        try {
            const [node, setNode] = createSignal<MarkdownNode>({ type: "markdown", id: "md-1", content: "" });
            const { container } = render(() => <MarkdownBlock node={node()} />);
            let prev = 0;
            const drops: string[] = [];
            for (let i = 1; i <= text.length; i += 3) {
                setNode({ type: "markdown", id: "md-1", content: text.slice(0, i) });
                vi.advanceTimersByTime(PAUSE_MS); // past the settle window: the settled render runs
                const len = (container.textContent ?? "").length;
                if (len < prev - SLACK_CHARS) drops.push(`after ${JSON.stringify(text.slice(0, i).slice(-14))}: ${prev} -> ${len}`);
                prev = len;
            }
            expect(drops).toEqual([]);
        } finally {
            vi.useRealTimers();
        }
    });
});
