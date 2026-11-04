// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Characterisation tests for <Markdown> itself: what it takes as input, how its
 * parts render (headings, tables, code, diagrams, content blocks), the table of
 * contents, the style and class options, the frozen prefix and open tail it keeps
 * between commits, and what it does when a render throws. The streaming-work
 * counters and the tail kill switch have files of their own.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, onCleanup } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    /** Mermaid instances mounted and torn down, and the charts they were given. */
    mermaidMounted: [] as string[],
    mermaidCleaned: 0,
    mermaidThrows: false,
    /** The hast → DOM render throws: at once ("early"), or after its components have been built ("late"). */
    renderThrows: false as false | "early" | "late",
    marks: [] as [string, string, string | undefined][],
}));

vi.mock("./markdown-mermaid", () => ({
    Mermaid: (p: { chart: string }) => {
        if (h.mermaidThrows) throw new Error("mermaid boom");
        h.mermaidMounted.push(p.chart);
        onCleanup(() => {
            h.mermaidCleaned++;
        });
        return <div class="mermaid-stub" data-chart={p.chart} />;
    },
    MermaidErrorFallback: (p: { chart: string }) => <div class="mermaid-fallback" data-chart={p.chart} />,
}));

vi.mock("hast-util-to-jsx-runtime", async (importOriginal) => {
    const real = await importOriginal<typeof import("hast-util-to-jsx-runtime")>();
    return {
        ...real,
        toJsxRuntime: (...args: Parameters<typeof real.toJsxRuntime>) => {
            const out = real.toJsxRuntime(...args);
            if (h.renderThrows === "early") throw new Error("render boom");
            if (h.renderThrows === "late") {
                // A hyper element, as the real runtime returns: building it creates the components, and then it throws.
                const late = () => {
                    let v: unknown = out;
                    while (typeof v === "function") v = (v as () => unknown)();
                    throw new Error("late render boom");
                };
                return Object.assign(late, { [Symbol("hyper-element")]: true }) as never;
            }
            return out;
        },
    };
});

vi.mock("@/perf", () => ({
    markStart: (name: string) => {
        h.marks.push(["start", name, undefined]);
    },
    markEnd: (name: string, detail?: string) => {
        h.marks.push(["end", name, detail]);
    },
}));

import { Markdown, __markdownRenderStats, __resetMarkdownRenderStats } from "./markdown";
import type { MarkdownMediaOpts } from "./markdown-media";

/** Long enough to cross the incremental splitter's minimum prefix, so a document renders in segments. */
const FILLER = "Filler prose that pads the document past the split threshold.\n\n".repeat(14);

beforeEach(() => {
    h.mermaidMounted.length = 0;
    h.mermaidCleaned = 0;
    h.mermaidThrows = false;
    h.renderThrows = false;
    h.marks.length = 0;
    __resetMarkdownRenderStats();
});

afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
});

const q = (c: HTMLElement, sel: string) => c.querySelector<HTMLElement>(sel)!;
const qa = (c: HTMLElement, sel: string) => Array.from(c.querySelectorAll<HTMLElement>(sel));

describe("Markdown — input", () => {
    it("takes the text atom over the text prop, and follows the atom", () => {
        const [atom, setAtom] = createSignal("from the atom");
        const { container } = render(() => <Markdown textAtom={atom} text="from the prop" />);
        expect(container.textContent).toContain("from the atom");
        expect(container.textContent).not.toContain("from the prop");
        setAtom("changed atom");
        expect(container.textContent).toContain("changed atom");
    });

    it("falls back to the text prop, and renders an empty document without either", () => {
        const { container, unmount } = render(() => <Markdown text="# prop heading" />);
        expect(q(container, ".heading").textContent).toBe("prop heading");
        unmount();
        const empty = render(() => <Markdown />);
        expect(q(empty.container, ".markdown-content-inner").textContent).toBe("");
    });
});

describe("Markdown — table of contents", () => {
    const DOC = "# Title\n\ntext\n\n## Two\n\ntext\n\n### Three\n\n#### Four\n";

    it("is absent unless asked for", () => {
        const { container } = render(() => <Markdown text={DOC} />);
        expect(container.querySelector(".toc")).toBeNull();
        cleanup();
        const off = render(() => <Markdown text={DOC} showTocAtom={() => false} />);
        expect(off.container.querySelector(".toc")).toBeNull();
    });

    it("lists the headings in order, indented by depth", () => {
        const { container } = render(() => <Markdown text={DOC} showTocAtom={() => true} />);
        expect(q(container, ".toc h4").textContent).toBe("Table of Contents");
        const items = qa(container, ".toc-item");
        expect(items.map((i) => i.textContent)).toEqual(["Two", "Three", "Four"]); // the title (h1) is not listed
        expect(items.map((i) => i.style.getPropertyValue("--indent-factor"))).toEqual(["2", "3", "4"]);
        expect(container.querySelector(".toc-empty")).toBeNull();
    });

    it("says so when there are no headings, and only when it is shown", () => {
        const { container } = render(() => <Markdown text="just prose" showTocAtom={() => true} />);
        expect(q(container, ".toc-empty").textContent).toBe("No sub-headings found");
        expect(q(container, ".toc-empty").style.getPropertyValue("--indent-factor")).toBe("2");
        expect(qa(container, ".toc")).toHaveLength(1); // the empty note stands in for the list, not beside it
        cleanup();
        const hidden = render(() => <Markdown text="just prose" showTocAtom={() => false} />);
        expect(hidden.container.querySelector(".toc")).toBeNull();
    });

    it("follows the text as it changes, keeping only the current headings", () => {
        const [text, setText] = createSignal("## First\n");
        const { container } = render(() => <Markdown text={text()} showTocAtom={() => true} />);
        expect(qa(container, ".toc-item").map((i) => i.textContent)).toEqual(["First"]);
        setText("## Second\n\n### Third\n");
        expect(qa(container, ".toc-item").map((i) => i.textContent)).toEqual(["Second", "Third"]);
        setText("no headings now");
        expect(q(container, ".toc-empty").textContent).toBe("No sub-headings found");
    });

    it("scrolls the content to a heading when its entry is clicked", () => {
        const { container } = render(() => <Markdown text={DOC} showTocAtom={() => true} />);
        const content = q(container, ".content");
        const scrollBy = vi.fn();
        content.scrollBy = scrollBy as unknown as typeof content.scrollBy;
        const two = qa(container, ".heading").find((e) => e.textContent === "Two")!;
        const rect = (top: number) => ({ top }) as DOMRect;
        vi.spyOn(content, "getBoundingClientRect").mockReturnValue(rect(100));
        vi.spyOn(two, "getBoundingClientRect").mockReturnValue(rect(340));
        expect(scrollBy).not.toHaveBeenCalled();
        fireEvent.click(qa(container, ".toc-item").find((e) => e.textContent === "Two")!);
        expect(scrollBy).toHaveBeenCalledTimes(1);
        expect(scrollBy).toHaveBeenCalledWith({ top: 240 });
    });

    it("scrolls for an in-page link, and not for a heading that isn't there", () => {
        const { container } = render(() => <Markdown text={"# One\n\n[jump](#one) and [lost](#nowhere)\n"} />);
        const content = q(container, ".content");
        const scrollBy = vi.fn();
        content.scrollBy = scrollBy as unknown as typeof content.scrollBy;
        const one = qa(container, ".heading")[0];
        vi.spyOn(content, "getBoundingClientRect").mockReturnValue({ top: 10 } as DOMRect);
        vi.spyOn(one, "getBoundingClientRect").mockReturnValue({ top: 60 } as DOMRect);
        const links = qa(container, "a");
        const click = (a: HTMLElement) => a.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
        expect(click(links[0])).toBe(false); // default prevented: no navigation
        expect(scrollBy).toHaveBeenCalledWith({ top: 50 });
        scrollBy.mockClear();
        click(links[1]);
        expect(scrollBy).not.toHaveBeenCalled();
    });

    it("does nothing when a heading is chosen in a non-scrolling document", () => {
        const errors: unknown[] = [];
        const onError = (e: ErrorEvent) => {
            errors.push(e.error);
            e.preventDefault();
        };
        window.addEventListener("error", onError);
        try {
            const { container } = render(() => <Markdown text={DOC} showTocAtom={() => true} scrollable={false} />);
            fireEvent.click(qa(container, ".toc-item")[1]);
        } finally {
            window.removeEventListener("error", onError);
        }
        expect(errors).toEqual([]);
    });
});

describe("Markdown — headings and paragraphs", () => {
    it("renders each level with its own class, and gives it an id under this instance's prefix", () => {
        const text = ["# a", "## b", "### c", "#### d", "##### e", "###### f"].join("\n\n");
        const { container } = render(() => <Markdown text={text} />);
        const heads = qa(container, ".heading");
        expect(heads.map((e) => e.className)).toEqual([1, 2, 3, 4, 5, 6].map((n) => `heading is-${n}`));
        expect(heads.map((e) => e.textContent)).toEqual(["a", "b", "c", "d", "e", "f"]);
        const prefix = heads[0].id.slice(0, -1);
        expect(prefix).toMatch(/^[0-9a-f-]{36}$/);
        expect(heads.map((e) => e.id)).toEqual(["a", "b", "c", "d", "e", "f"].map((s) => prefix + s));
    });

    it("gives two instances different id prefixes", () => {
        const a = render(() => <Markdown text="# same" />);
        const b = render(() => <Markdown text="# same" />);
        expect(q(a.container, ".heading").id).not.toBe(q(b.container, ".heading").id);
    });

    it("wraps a paragraph in div.paragraph", () => {
        const { container } = render(() => <Markdown text={"one\n\ntwo"} />);
        expect(qa(container, "div.paragraph").map((e) => e.textContent)).toEqual(["one", "two"]);
    });

    it("turns a bare URL into a link", () => {
        const { container } = render(() => <Markdown text="see https://example.com/page now" />);
        expect(q(container, "a").getAttribute("href")).toBe("https://example.com/page");
    });

    it("links a bare domain, and not one inside code", () => {
        const { container } = render(() => <Markdown text={"visit example.org/docs or `example.net/x`"} />);
        expect(qa(container, "a").map((a) => a.textContent)).toEqual(["example.org/docs"]);
    });
});

describe("Markdown — tables", () => {
    const TABLE = "| a | b | c |\n|:--|:-:|--:|\n| 1 | 2 | 3 |\n";

    it("styles the cells and rows, and aligns the columns it was told to", () => {
        const { container } = render(() => <Markdown text={TABLE} />);
        const ths = qa(container, "th");
        expect(ths).toHaveLength(3);
        for (const th of ths) {
            expect(th.className).toContain("uppercase");
            expect(th.className).toContain("text-primary");
        }
        expect(ths[0].classList.contains("text-left")).toBe(true);
        expect(ths[1].classList.contains("text-center")).toBe(true);
        expect(ths[1].classList.contains("text-left")).toBe(false);
        expect(ths[2].classList.contains("text-right")).toBe(true);
        expect(ths[2].classList.contains("text-left")).toBe(false);
        const tds = qa(container, "td");
        for (const td of tds) expect(td.className).toContain("text-secondary");
        expect(tds[0].className).toContain("px-3");
        expect(tds[1].classList.contains("text-center")).toBe(true);
        expect(tds[2].classList.contains("text-right")).toBe(true);
        expect(q(container, "thead").className).toContain("bg-hover");
        expect(qa(container, "tr").every((r) => r.className.includes("border-b"))).toBe(true);
        expect(q(container, "tbody").textContent).toBe("123");
    });

    it("keeps the spans of a raw HTML table", () => {
        const html =
            '<table><thead><tr><th colspan="2" scope="col">H</th></tr></thead><tbody><tr><td rowspan="2" colspan="1">x</td><td>y</td></tr></tbody></table>';
        const { container } = render(() => <Markdown text={html} />);
        expect(q(container, "th").getAttribute("colspan")).toBe("2");
        expect(q(container, "th").getAttribute("scope")).toBe("col");
        expect(q(container, "td").getAttribute("rowspan")).toBe("2");
    });

    it("maps align attributes of a raw HTML table to classes", () => {
        const html = '<table><tbody><tr><td align="right">r</td><th align="center">c</th></tr></tbody></table>';
        const { container } = render(() => <Markdown text={html} />);
        expect(q(container, "td").classList.contains("text-right")).toBe(true);
        expect(q(container, "th").classList.contains("text-center")).toBe(true);
        expect(q(container, "th").classList.contains("text-left")).toBe(false);
    });

    it("wraps a table in the table block", () => {
        const { container } = render(() => <Markdown text={TABLE} />);
        expect(q(container, "table").parentElement).not.toBe(q(container, ".markdown-content-inner"));
    });
});

describe("Markdown — code", () => {
    it("offers a run button on a fenced block only when told what to run", () => {
        const text = "```bash\necho hi\n```\n";
        const { container, unmount } = render(() => <Markdown text={text} />);
        expect(container.querySelector(".codeblock-actions .iconbutton, .codeblock-actions [data-icon]")).toBeNull();
        const without = qa(container, ".codeblock-actions button").length;
        unmount();
        const run = vi.fn();
        const withRun = render(() => <Markdown text={text} onClickExecute={run} />);
        const buttons = qa(withRun.container, ".codeblock-actions button");
        expect(buttons.length).toBe(without + 1);
        fireEvent.click(buttons[buttons.length - 1]);
        expect(run).toHaveBeenCalledWith("echo hi");
    });

    it("renders inline code as code", () => {
        const { container } = render(() => <Markdown text="use `x = 1` here" />);
        expect(q(container, "code").textContent).toBe("x = 1");
    });
});

describe("Markdown — links", () => {
    it("keeps file and agentmux links, and drops a javascript one", () => {
        const text = "[f](file:///tmp/a.txt) [t](agentmux://task/12) [j](javascript:alert(1))";
        const { container } = render(() => <Markdown text={text} />);
        const hrefs = qa(container, "a").map((a) => a.getAttribute("href"));
        expect(hrefs).toContain("file:///tmp/a.txt");
        expect(hrefs).toContain("agentmux://task/12");
        expect(hrefs.some((x) => x?.startsWith("javascript"))).toBe(false);
    });

    it("keeps a data: image src and a file: one", () => {
        const { container } = render(() => (
            <Markdown text={"![d](data:image/png;base64,AAAA)"} media={{} as MarkdownMediaOpts} />
        ));
        expect(container.innerHTML).toContain("data:image/png;base64,AAAA");
    });
});

describe("Markdown — diagrams and content blocks", () => {
    it("hands a mermaid fence's text to the diagram", () => {
        const { container } = render(() => <Markdown text={"```mermaid\ngraph TD\n  A-->B\n```\n"} />);
        expect(h.mermaidMounted).toEqual(["graph TD\n  A-->B"]);
        expect(q(container, ".mermaid-stub").dataset.chart).toBe("graph TD\n  A-->B");
    });

    it("shows the fallback with the chart text when the diagram throws", () => {
        h.mermaidThrows = true;
        const { container } = render(() => <Markdown text={"```mermaid\ngraph TD\n  A-->B\n```\n"} />);
        expect(container.querySelector(".mermaid-stub")).toBeNull();
        expect(q(container, ".mermaid-fallback").dataset.chart).toBe("graph TD\n  A-->B");
    });

    it("shows a content block as its name and size", () => {
        const body = "x".repeat(2048);
        const text = `@@@start code "notes.txt"\n${body}\n@@@end code "notes.txt"\n`;
        const { container } = render(() => <Markdown text={text} />);
        expect(q(container, ".wave-block-filename").textContent).toBe("notes.txt");
        expect(q(container, ".wave-block-size").textContent).toBe("2 KB");
    });
});

describe("Markdown — style and shape", () => {
    it("merges the style prop with the font-size overrides, bounded to 6-64 px", () => {
        const { container } = render(() => (
            <Markdown text="x" style={{ color: "red" }} fontSizeOverride={100} fixedFontSizeOverride={2} />
        ));
        const el = q(container, ".markdown");
        expect(el.style.color).toBe("red");
        expect(el.style.getPropertyValue("--markdown-font-size")).toBe("64px");
        expect(el.style.getPropertyValue("--markdown-fixed-font-size")).toBe("6px");
    });

    it("bounds the fixed font size from above too", () => {
        const { container } = render(() => <Markdown text="x" fixedFontSizeOverride={100} />);
        expect(q(container, ".markdown").style.getPropertyValue("--markdown-fixed-font-size")).toBe("64px");
    });

    it("passes the overrides through as given inside the bounds, and sets none when absent", () => {
        const a = render(() => <Markdown text="x" fontSizeOverride={6} fixedFontSizeOverride={64} />);
        expect(q(a.container, ".markdown").style.getPropertyValue("--markdown-font-size")).toBe("6px");
        expect(q(a.container, ".markdown").style.getPropertyValue("--markdown-fixed-font-size")).toBe("64px");
        const b = render(() => <Markdown text="x" fontSizeOverride={13} fixedFontSizeOverride={14} />);
        expect(q(b.container, ".markdown").style.getPropertyValue("--markdown-font-size")).toBe("13px");
        expect(q(b.container, ".markdown").style.getPropertyValue("--markdown-fixed-font-size")).toBe("14px");
        const c = render(() => <Markdown text="x" />);
        expect(q(c.container, ".markdown").getAttribute("style") ?? "").toBe("");
    });

    it("scrolls its content by default, with the class names it was given", () => {
        const { container } = render(() => <Markdown text="x" class="mine" contentClass="inner" />);
        const root = q(container, ".markdown");
        expect(root.classList.contains("mine")).toBe(true);
        const content = q(container, ".content");
        expect(content.classList.contains("inner")).toBe(true);
        expect(content.classList.contains("non-scrollable")).toBe(false);
        expect(content.firstElementChild!.className).toBe("markdown-content-inner");
    });

    it("renders straight into a non-scrolling content when asked", () => {
        const { container } = render(() => <Markdown text="x" scrollable={false} contentClass="inner" />);
        const content = q(container, ".content");
        expect(content.classList.contains("non-scrollable")).toBe(true);
        expect(content.classList.contains("inner")).toBe(true);
        expect(container.querySelector(".markdown-content-inner")).toBeNull();
    });

    it("passes safe raw HTML and slugs headings, and drops a script", () => {
        const { container } = render(() => <Markdown text={"# Head\n\n<b>bold</b> and <script>1</script>"} />);
        expect(q(container, "b").textContent).toBe("bold");
        expect(container.querySelector("script")).toBeNull();
        expect(q(container, ".heading").id.endsWith("head")).toBe(true);
    });

    it("leaves raw HTML and heading ids to the plain parse when the HTML plugins are off", () => {
        const { container } = render(() => (
            <Markdown text={"# Head\n\n<b>bold</b> and <script>1</script>"} rehype={false} />
        ));
        expect(container.querySelector("b")).toBeNull();
        expect(container.querySelector("script")).toBeNull();
        expect(q(container, ".heading").id).toBe("");
    });
});

describe("Markdown — the frozen prefix and the open tail", () => {
    const doc = (n: number) => `${FILLER}tail line ${n}`;

    it("renders each frozen segment once as the text grows, and counts its work", () => {
        const [text, setText] = createSignal(doc(1));
        const { container } = render(() => <Markdown text={text()} streaming />);
        const segmentsAfterFirst = __markdownRenderStats.domSegmentRenders;
        expect(segmentsAfterFirst).toBeGreaterThan(0);
        const frozenNode = q(container, "div.paragraph");
        for (let n = 2; n <= 6; n++) setText(doc(n));
        expect(__markdownRenderStats.domSegmentRenders).toBe(segmentsAfterFirst);
        expect(q(container, "div.paragraph")).toBe(frozenNode);
        expect(container.textContent).toContain("tail line 6");
        expect(container.textContent).not.toContain("tail line 5");
        expect(__markdownRenderStats.commits).toBe(6);
        expect(__markdownRenderStats.parsedChars).toBeGreaterThan(0);
    });

    it("starts over when the text is replaced rather than extended", () => {
        const [text, setText] = createSignal(`${"Alpha line.\n\n".repeat(40)}tail one`);
        const { container } = render(() => <Markdown text={text()} />);
        expect(container.textContent).toContain("Alpha line.");
        setText(`${"Bravo line.\n\n".repeat(40)}tail two`);
        expect(container.textContent).not.toContain("Alpha");
        expect(container.textContent).toContain("Bravo line.");
        expect(qa(container, "div.paragraph")).toHaveLength(41);
    });

    it("re-renders the frozen prefix when a content block's body changes behind the same placeholder", () => {
        const block = (body: string) => `@@@start code "f.txt"\n${body}\n@@@end code "f.txt"\n\n`;
        const [body, setBody] = createSignal("y".repeat(1024));
        const { container } = render(() => <Markdown text={block(body()) + FILLER + "tail"} />);
        expect(q(container, ".wave-block-size").textContent).toBe("1 KB");
        setBody("y".repeat(3072));
        expect(q(container, ".wave-block-size").textContent).toBe("3 KB");
    });

    it("re-renders the frozen prefix when highlighting is turned off", () => {
        const text = "```ts\nconst a = 1;\n```\n\n" + FILLER + "tail";
        const [highlight, setHighlight] = createSignal(true);
        const { container } = render(() => <Markdown text={text} highlight={highlight()} />);
        const before = q(container, "pre.codeblock");
        setHighlight(false);
        expect(q(container, "pre.codeblock")).not.toBe(before);
    });

    it("keeps one processor for the plain tail when streaming with highlighting, and none extra otherwise", () => {
        const [text, setText] = createSignal(doc(1));
        render(() => <Markdown text={text()} streaming />);
        for (let n = 2; n <= 5; n++) setText(doc(n));
        expect(__markdownRenderStats.processorBuilds).toBe(2); // the final one, and the plain one for the tail
        cleanup();
        __resetMarkdownRenderStats();
        const [t2, setT2] = createSignal(doc(1));
        render(() => <Markdown text={t2()} streaming highlight={false} />);
        for (let n = 2; n <= 5; n++) setT2(doc(n));
        expect(__markdownRenderStats.processorBuilds).toBe(1); // no highlighting: the tail needs no plain one
        cleanup();
        __resetMarkdownRenderStats();
        const [t3, setT3] = createSignal(doc(1));
        render(() => <Markdown text={t3()} />);
        for (let n = 2; n <= 5; n++) setT3(doc(n));
        expect(__markdownRenderStats.processorBuilds).toBe(1); // not streaming: one processor for everything
    });

    it("marks a fenced block for colour unless it is still streaming or colour is off", () => {
        const fence = "```ts\nconst a = 1;\n```\n";
        const coloured = (c: HTMLElement) => qa(c, "code").map((e) => e.classList.contains("sh-shiki"));
        const settled = render(() => <Markdown text={fence} />);
        expect(coloured(settled.container)).toEqual([true]);
        cleanup();
        const streaming = render(() => <Markdown text={fence} streaming />);
        expect(coloured(streaming.container)).toEqual([false]);
        cleanup();
        const plain = render(() => <Markdown text={fence} highlight={false} />);
        expect(coloured(plain.container)).toEqual([false]);
        cleanup();
        // Streaming: the closed block in the frozen prefix is coloured, the open block in the tail isn't.
        const both = render(() => <Markdown text={`${fence}\n${FILLER}${fence}`} streaming />);
        expect(coloured(both.container)).toEqual([true, false]);
    });

    it("holds back a half-typed inline tag only while streaming", () => {
        const text = `${FILLER}colour <span class="am-ok`;
        const streaming = render(() => <Markdown text={text} streaming />);
        expect(streaming.container.textContent).not.toContain("<span");
        cleanup();
        const settled = render(() => <Markdown text={text} streaming={false} />);
        expect(settled.container.textContent).toContain("<span");
    });

    it("holds back a half-typed tag in a short document too", () => {
        const { container } = render(() => <Markdown text={`go <span class="am-ok`} streaming />);
        expect(container.textContent).not.toContain("<span");
        expect(container.textContent).toContain("go");
    });

    it("tears down the components of its frozen prefix and its tail when it unmounts", () => {
        const fence = "```mermaid\ngraph TD\n  A-->B\n```\n\n";
        const text = fence + FILLER + fence.replace("A-->B", "C-->D");
        const { unmount } = render(() => <Markdown text={text} />);
        expect(h.mermaidMounted).toHaveLength(2);
        expect(h.mermaidCleaned).toBe(0);
        unmount();
        expect(h.mermaidCleaned).toBe(2);
    });

    it("tears down every frozen segment on unmount, not just the last one", () => {
        const fence = (n: string) => `\`\`\`mermaid\ngraph TD\n  ${n}-->Z\n\`\`\`\n\n`;
        const [text, setText] = createSignal(fence("A") + FILLER + "tail");
        const { unmount } = render(() => <Markdown text={text()} />);
        setText(fence("A") + FILLER + fence("B") + FILLER + "tail");
        setText(fence("A") + FILLER + fence("B") + FILLER + fence("C") + FILLER + "tail");
        expect(__markdownRenderStats.domSegmentRenders).toBeGreaterThanOrEqual(3);
        const mounted = h.mermaidMounted.length;
        expect(mounted).toBeGreaterThanOrEqual(3);
        unmount();
        expect(h.mermaidCleaned).toBe(mounted);
    });

    it("tears down the old tail when the tail is rebuilt, and when the frozen prefix is invalidated", () => {
        const fence = "```mermaid\ngraph TD\n  A-->B\n```\n";
        const [text, setText] = createSignal(FILLER + fence);
        render(() => <Markdown text={text()} streaming />);
        expect(h.mermaidMounted).toHaveLength(1);
        setText(FILLER + fence + "\nmore after it");
        expect(h.mermaidCleaned).toBeGreaterThanOrEqual(1);
        const cleanedBefore = h.mermaidCleaned;
        setText(`${"Other line.\n\n".repeat(40)}` + fence);
        expect(h.mermaidCleaned).toBeGreaterThan(cleanedBefore);
    });

    it("patches the open tail's last text run in place while it only grows", () => {
        const [text, setText] = createSignal(`${FILLER}growing`);
        const { container } = render(() => <Markdown text={text()} streaming />);
        const tailPara = qa(container, "div.paragraph").pop()!;
        const rebuilds = __markdownRenderStats.tailRebuilds;
        setText(`${FILLER}growing more`);
        setText(`${FILLER}growing more still`);
        expect(qa(container, "div.paragraph").pop()).toBe(tailPara);
        expect(tailPara.textContent).toBe("growing more still");
        expect(__markdownRenderStats.tailRebuilds).toBe(rebuilds);
        expect(__markdownRenderStats.tailInPlaceUpdates).toBe(2);
    });

    it("rebuilds the tail when an earlier run of its text changes, or its shape does", () => {
        const [text, setText] = createSignal(`${FILLER}- one\n- two`);
        const { container } = render(() => <Markdown text={text()} streaming />);
        const rebuilds = () => __markdownRenderStats.tailRebuilds;
        const base = rebuilds();
        setText(`${FILLER}- ONE\n- two`); // an earlier run changed
        expect(rebuilds()).toBe(base + 1);
        setText(`${FILLER}- ONE\n- two\n- three`); // a new item: the shape changed
        expect(rebuilds()).toBe(base + 2);
        expect(qa(container, "li").map((e) => e.textContent)).toEqual(["ONE", "two", "three"]);
    });

    it("keeps a tail heading in the table of contents while the paragraph under it is patched", () => {
        const [text, setText] = createSignal(`${FILLER}## Head\ngrowing`);
        const { container } = render(() => <Markdown text={text()} streaming showTocAtom={() => true} />);
        setText(`${FILLER}## Head\ngrowing more`);
        expect(__markdownRenderStats.tailInPlaceUpdates).toBe(1);
        expect(qa(container, ".toc-item").map((e) => e.textContent)).toEqual(["Head"]);
    });

    it("rebuilds rather than patching when the run to patch is a different one", () => {
        const [text, setText] = createSignal(`${FILLER}<span>a</span><span> </span>`);
        const { container } = render(() => <Markdown text={text()} streaming />);
        setText(`${FILLER}<span>a</span><span>b</span>`);
        expect(qa(container, "div.paragraph").pop()!.textContent).toBe("ab");
        expect(__markdownRenderStats.tailInPlaceUpdates).toBe(0);
    });

    it("never patches heading text in place, so its id and entry follow", () => {
        const [text, setText] = createSignal(`${FILLER}## Head`);
        const { container } = render(() => <Markdown text={text()} streaming showTocAtom={() => true} />);
        const base = __markdownRenderStats.tailRebuilds;
        setText(`${FILLER}## Header`);
        expect(__markdownRenderStats.tailRebuilds).toBe(base + 1);
        expect(__markdownRenderStats.tailInPlaceUpdates).toBe(0);
        expect(qa(container, ".toc-item").map((e) => e.textContent)).toEqual(["Header"]);
        expect(q(container, ".heading").id.endsWith("header")).toBe(true);
    });

    it("rebuilds the tail when its text node has left the page", () => {
        const [text, setText] = createSignal(`${FILLER}growing`);
        const { container } = render(() => <Markdown text={text()} streaming />);
        setText(`${FILLER}growing a`); // locates and patches the node
        const tailPara = qa(container, "div.paragraph").pop()!;
        tailPara.textContent = "replaced by someone else"; // the old text node is detached
        const base = __markdownRenderStats.tailRebuilds;
        setText(`${FILLER}growing a bit`);
        expect(__markdownRenderStats.tailRebuilds).toBe(base + 1);
        expect(qa(container, "div.paragraph").pop()!.textContent).toBe("growing a bit");
    });

    it("lists the headings of the frozen prefix and the tail together", () => {
        const [text, setText] = createSignal(`## Early\n\n${FILLER}## Late`);
        const { container } = render(() => <Markdown text={text()} showTocAtom={() => true} streaming />);
        expect(qa(container, ".toc-item").map((e) => e.textContent)).toEqual(["Early", "Late"]);
        setText(`## Early\n\n${FILLER}## Late\n\ntext\n\n### Later`);
        expect(qa(container, ".toc-item").map((e) => e.textContent)).toEqual(["Early", "Late", "Later"]);
    });
});

describe("Markdown — a render that throws", () => {
    it("shows the raw text, logs, and has no table of contents", () => {
        const error = vi.spyOn(console, "error").mockImplementation(() => {});
        h.renderThrows = "early";
        const { container } = render(() => <Markdown text={"# Title\n\nbody"} showTocAtom={() => true} />);
        expect(q(container, ".markdown-content-inner pre").textContent).toBe("# Title\n\nbody");
        expect(error).toHaveBeenCalledWith("Markdown render error:", expect.any(Error));
        expect(q(container, ".toc-empty").textContent).toBe("No sub-headings found");
    });

    it("recovers on the next commit, and builds on no cache from the failure", () => {
        vi.spyOn(console, "error").mockImplementation(() => {});
        const [text, setText] = createSignal(`${FILLER}tail`);
        const { container } = render(() => <Markdown text={text()} />);
        h.renderThrows = "early";
        setText(`${FILLER}tail\n\nbroken`); // a new paragraph: the tail is rebuilt, not patched
        expect(container.querySelector("pre")!.textContent).toBe(`${FILLER}tail\n\nbroken`);
        h.renderThrows = false;
        const segments = __markdownRenderStats.domSegmentRenders;
        setText(`${FILLER}tail\n\nfixed`);
        expect(container.querySelector("pre")).toBeNull();
        expect(container.textContent).toContain("fixed");
        expect(__markdownRenderStats.domSegmentRenders).toBeGreaterThan(segments); // the frozen prefix was rebuilt
    });

    it("tears down what a failed render built before it threw", () => {
        vi.spyOn(console, "error").mockImplementation(() => {});
        h.renderThrows = "late";
        render(() => <Markdown text={"```mermaid\ngraph TD\n  A-->B\n```\n"} />);
        expect(h.mermaidMounted).toHaveLength(1);
        expect(h.mermaidCleaned).toBe(1);
    });

    it("tears down a failed frozen segment too", () => {
        vi.spyOn(console, "error").mockImplementation(() => {});
        h.renderThrows = "late";
        render(() => <Markdown text={"```mermaid\ngraph TD\n  A-->B\n```\n\n" + FILLER + "tail"} />);
        expect(h.mermaidMounted.length).toBeGreaterThan(0);
        expect(h.mermaidCleaned).toBe(h.mermaidMounted.length);
    });

    it("drops the frozen prefix after a failure, so the next render builds it afresh", () => {
        vi.spyOn(console, "error").mockImplementation(() => {});
        const [text, setText] = createSignal(`${FILLER}- a\n- b`);
        render(() => <Markdown text={text()} />);
        const segments = __markdownRenderStats.domSegmentRenders;
        h.renderThrows = "early";
        setText(`${FILLER}- a\n- b\n- c`); // the split doesn't move; only the tail is rebuilt, and it throws
        h.renderThrows = false;
        setText(`${FILLER}- a\n- b\n- d`);
        expect(__markdownRenderStats.domSegmentRenders).toBe(segments + 1);
    });

    it("drops the tail after a failure that happened in a frozen segment", () => {
        vi.spyOn(console, "error").mockImplementation(() => {});
        const fence = "```mermaid\ngraph TD\n  A-->B\n```\n";
        const [text, setText] = createSignal(FILLER + fence);
        render(() => <Markdown text={text()} />);
        expect(h.mermaidMounted).toHaveLength(1);
        expect(h.mermaidCleaned).toBe(0);
        h.renderThrows = "early";
        setText(`${FILLER}${fence}\nmore`); // the split moves past the fence: a new segment, which throws
        expect(h.mermaidCleaned).toBe(1);
    });

    it("marks the start and the end of every render, even a failed one", () => {
        vi.spyOn(console, "error").mockImplementation(() => {});
        render(() => <Markdown text="hello" />);
        expect(h.marks).toEqual([
            ["start", "markdown-render", undefined],
            ["end", "markdown-render", "len=5"],
        ]);
        h.marks.length = 0;
        h.renderThrows = "early";
        render(() => <Markdown text="hello!" />);
        expect(h.marks).toEqual([
            ["start", "markdown-render", undefined],
            ["end", "markdown-render", "len=6"],
        ]);
    });
});
