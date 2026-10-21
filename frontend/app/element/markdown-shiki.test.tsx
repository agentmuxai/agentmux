// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Code in markdown coloured by shiki
 *  (SPEC_MARKDOWN_CODE_ON_SHIKI_AND_REDUNDANT_LIBRARIES_2026_10_21.md §4). */

import { cleanup, render, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const shiki = vi.hoisted(() => ({ calls: [] as [string, string][] }));
vi.mock("@/app/view/agent/components/shell-highlight/embedded-highlight", () => ({
    cachedRuns: () => undefined,
    highlightBody: async (text: string, lang: string) => {
        shiki.calls.push([lang, text]);
        if (lang === "nosuchlang") return null;
        return [{ text: "const", style: { "--shiki-dark": "#f00", "--shiki-light": "#a00" } }, { text: text.slice(5) }];
    },
}));

import { Markdown } from "./markdown";
import { codeLanguage, rehypeMarkCodeLang } from "./rehype-mark-code-lang";

afterEach(() => {
    cleanup();
    shiki.calls.length = 0;
});

const block = (lang: string, code: string) => "```" + lang + "\n" + code + "\n```";

describe("markdown code on shiki", () => {
    it("shows a fenced block plain at once, then coloured, with the same text", async () => {
        const c = render(() => <Markdown text={block("ts", "const a = 1;")} scrollable={false} />).container;
        const code = c.querySelector("pre code.sh-shiki")!;
        expect(code).not.toBeNull();
        expect(code.textContent).toBe("const a = 1;\n"); // a fenced block ends in a newline; Copy trims it
        await waitFor(() => expect(code.querySelector("span")).not.toBeNull());
        expect(code.textContent).toBe("const a = 1;\n"); // a fenced block ends in a newline; Copy trims it
        expect((code.querySelector("span") as HTMLElement).style.getPropertyValue("--shiki-dark")).toBe("#f00");
        expect(shiki.calls).toEqual([["ts", "const a = 1;\n"]]);
    });

    it("leaves inline code and a block without a language plain", () => {
        const c = render(() => <Markdown text={"Run `ls` now.\n\n" + block("", "plain text")} scrollable={false} />).container;
        expect(c.querySelector(".sh-shiki")).toBeNull();
        expect(shiki.calls).toEqual([]);
    });

    it("doesn't colour while highlighting is off (a streaming tail)", () => {
        const c = render(() => <Markdown text={block("ts", "const a = 1;")} highlight={false} scrollable={false} />).container;
        expect(c.querySelector(".sh-shiki")).toBeNull();
        expect(shiki.calls).toEqual([]);
    });

    it("keeps a language shiki can't colour plain", async () => {
        const c = render(() => <Markdown text={block("nosuchlang", "x = 1")} scrollable={false} />).container;
        await waitFor(() => expect(shiki.calls).toHaveLength(1));
        expect(c.querySelector("pre code span")).toBeNull();
        expect(c.querySelector("pre code")!.textContent).toBe("x = 1\n");
    });
});

describe("rehypeMarkCodeLang", () => {
    const code = (className: unknown) => ({ type: "element", tagName: "code", properties: { className }, children: [] });

    it("marks only a <code> directly inside a <pre>", () => {
        const fenced = code(["language-rust"]);
        const inline = code(["language-rust"]);
        const tree = {
            type: "root",
            children: [
                { type: "element", tagName: "pre", properties: {}, children: [fenced] },
                { type: "element", tagName: "p", properties: {}, children: [inline] },
            ],
        };
        rehypeMarkCodeLang()(tree);
        expect(fenced.properties).toMatchObject({ dataHighlight: "rust" });
        expect(inline.properties).not.toHaveProperty("dataHighlight");
    });

    it("reads the language from a class list or a class string", () => {
        expect(codeLanguage(code(["x", "language-py"]))).toBe("py");
        expect(codeLanguage(code("language-sh other"))).toBe("sh");
        expect(codeLanguage(code(["plain"]))).toBeUndefined();
    });
});
