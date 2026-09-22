// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Safety tests for incremental-render split points.
 *
 * The bar here is NOT "does it split often" — it is "does it ever split
 * somewhere that changes what the user reads." A missed split costs
 * performance; a wrong split corrupts output. So most of these assert that a
 * split is REFUSED.
 *
 * The final test is the real contract: for a range of documents, parsing
 * prefix+tail separately must produce the same HTML as parsing the whole thing.
 */

import { describe, expect, it } from "vitest";
import { unified } from "unified";
import remarkParse from "remark-parse";
import remarkGfm from "remark-gfm";
import remarkRehype from "remark-rehype";
import { findSafeSplitPoint } from "./markdown-incremental";

const PAD = "Filler paragraph that exists only to clear the minimum prefix length.\n\n".repeat(12);

describe("findSafeSplitPoint — refuses unsafe cuts", () => {
    it("never splits inside a fenced code block", () => {
        const text = `${PAD}\`\`\`ts\nconst a = 1;\n\nconst b = 2;\n\nconst c = 3;\n`;
        const at = findSafeSplitPoint(text);
        const fenceStart = text.indexOf("```");
        expect(at === -1 || at <= fenceStart).toBe(true);
    });

    it("never splits inside a tilde fence", () => {
        const text = `${PAD}~~~\nline\n\nline\n\nline\n`;
        const at = findSafeSplitPoint(text);
        const fenceStart = text.indexOf("~~~");
        expect(at === -1 || at <= fenceStart).toBe(true);
    });

    it("never splits inside an HTML comment", () => {
        const text = `${PAD}<!--\nnote\n\nmore note\n\nstill inside\n`;
        const at = findSafeSplitPoint(text);
        const commentStart = text.indexOf("<!--");
        expect(at === -1 || at <= commentStart).toBe(true);
    });

    it("refuses when link reference definitions are present", () => {
        expect(findSafeSplitPoint(`${PAD}[ref]: https://example.com\n\nText using [ref].\n`)).toBe(-1);
    });

    it("refuses when footnotes are present", () => {
        expect(findSafeSplitPoint(`${PAD}Text with a footnote[^1].\n\n[^1]: The note.\n`)).toBe(-1);
    });

    it("does not cut before a list continuation", () => {
        const text = `${PAD}- first\n\n- second\n\n- third\n`;
        const at = findSafeSplitPoint(text);
        if (at !== -1) expect(text.slice(at).startsWith("-")).toBe(false);
    });

    it("does not cut before a table row", () => {
        const text = `${PAD}| a | b |\n|---|---|\n\n| 1 | 2 |\n`;
        const at = findSafeSplitPoint(text);
        if (at !== -1) expect(text.slice(at).startsWith("|")).toBe(false);
    });

    it("does not cut before a setext underline or blockquote", () => {
        const text = `${PAD}Heading text\n\n===\n\n> quoted\n`;
        const at = findSafeSplitPoint(text);
        if (at !== -1) {
            const tail = text.slice(at);
            expect(tail.startsWith("=")).toBe(false);
            expect(tail.startsWith(">")).toBe(false);
        }
    });

    it("refuses on short documents", () => {
        expect(findSafeSplitPoint("# Title\n\nA paragraph.\n")).toBe(-1);
    });
});

describe("findSafeSplitPoint — still splits where it is safe", () => {
    it("splits ordinary prose separated by blank lines", () => {
        const text = `${PAD}## A heading\n\nA final paragraph.\n`;
        expect(findSafeSplitPoint(text)).toBeGreaterThan(0);
    });

    it("splits after a closed fence", () => {
        const text = `${PAD}\`\`\`ts\nconst a = 1;\n\`\`\`\n\nProse after the fence.\n\nMore prose.\n`;
        const at = findSafeSplitPoint(text);
        expect(at).toBeGreaterThan(text.indexOf("```"));
    });
});

/**
 * The contract that actually matters: a split must be semantically invisible.
 */
describe("split equivalence — prefix+tail renders identically to the whole", () => {
    const processor = unified()
        .use(remarkParse)
        .use(remarkGfm)
        .use(remarkRehype, { allowDangerousHtml: true });

    /**
     * Source offsets necessarily differ between a whole-document parse and a
     * split one, so they are stripped. Everything that decides what the user
     * actually sees — node types, tags, properties, text — is compared.
     * Compares hast children rather than HTML because merging hast children is
     * exactly what the renderer does.
     */
    const strip = (node: any): any => {
        if (Array.isArray(node)) return node.map(strip);
        if (node && typeof node === "object") {
            const out: any = {};
            for (const [k, v] of Object.entries(node)) {
                if (k === "position") continue;
                out[k] = strip(v);
            }
            return out;
        }
        return node;
    };

    /**
     * `remark-rehype` emits whitespace-only text nodes as separators BETWEEN
     * top-level block elements. A split loses exactly one of them at the seam.
     * That whitespace is insignificant between block-level elements — it
     * changes no rendered output — so it is normalized away here rather than
     * papered over in the implementation. Only TOP-LEVEL separators are
     * dropped; whitespace inside elements is still compared, because there it
     * can matter (e.g. inside `<pre>`).
     */
    const render = (src: string) =>
        strip((processor.runSync(processor.parse(src)) as any).children).filter(
            (n: any) => !(n.type === "text" && typeof n.value === "string" && n.value.trim() === ""),
        );

    const CASES: Record<string, string> = {
        prose: `${PAD}## Heading\n\nParagraph one.\n\nParagraph two with \`code\`.\n`,
        closedFence: `${PAD}\`\`\`js\nconst x = 1;\n\`\`\`\n\nAfter the fence.\n\nAnd more.\n`,
        openFence: `${PAD}\`\`\`js\nconst x = 1;\n\nconst y = 2;\n`,
        list: `${PAD}- one\n- two\n\n- three\n`,
        table: `${PAD}| a | b |\n|---|---|\n| 1 | 2 |\n\nAfter table.\n`,
        htmlComment: `${PAD}<!-- a\n\nb -->\n\nAfter comment.\n`,
        nestedList: `${PAD}1. first\n   - nested\n\n2. second\n`,
        blockquote: `${PAD}> quoted line\n>\n> still quoted\n\nAfter quote.\n`,
    };

    for (const [name, text] of Object.entries(CASES)) {
        it(`${name}: splitting does not change the rendered output`, () => {
            const at = findSafeSplitPoint(text);
            if (at === -1) return; // refused — trivially safe
            const combined = [...render(text.slice(0, at)), ...render(text.slice(at))];
            expect(combined).toEqual(render(text));
        });
    }
});
