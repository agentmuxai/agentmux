// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Streaming-render work invariants for <Markdown>.
 *
 * WHY THESE ASSERT ON COUNTS, NOT MILLISECONDS
 *
 * The regression being guarded is algorithmic, and the repo has already taken
 * the position (Discussion #1161) that absolute wall-clock gates on shared CI
 * runners are theater — they flake until someone disables them, which is worse
 * than no gate at all. So these assert on deterministic work counters
 * (`__markdownRenderStats`): machine-independent, non-flaky, and they fail for
 * a reason a reader can act on.
 *
 * WHAT WENT WRONG, SO NOBODY RE-INTRODUCES IT
 *
 * `<Markdown>` re-parses the ENTIRE text on every commit. During agent
 * streaming the text grows ~11 commits/sec, so rendering one 32KB response
 * burned 1.7s of main thread in blocks up to 92ms and starved keystrokes in the
 * composer — measured in
 * `docs/analysis/ANALYSIS_AGENT_PANE_TYPING_UNDER_LOAD_2026_09_22.md` §6, with
 * `tools/tests/bench-markdown-parse.mjs`. Separately, the unified processor was
 * being rebuilt inside the render memo, adding a fixed ~4ms to every one of
 * those commits.
 *
 * This is the third appearance of this bug class in this codebase (see the
 * "not silky" history in SPEC_TERM_DOUBLE_RAF_TEAROUT_2026_05_30). Twice it was
 * fixed, documented, and reintroduced anyway. Hence executable guards.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";

import { Markdown, __markdownRenderStats, __resetMarkdownRenderStats } from "./markdown";

/**
 * Long enough to cross the incremental splitter's minimum-prefix threshold, so
 * these render through the frozen-prefix path rather than the whole-document
 * fallback.
 */
const LONG_FILLER = "Filler prose that pads the document past the split threshold.\n\n".repeat(14);

afterEach(() => {
    cleanup();
});

/** Streams `text` in `chunks` slices, committing each one, like an agent response. */
function streamInto(text: string, chunks: number) {
    const [content, setContent] = createSignal("");
    const result = render(() => <Markdown text={content()} scrollable={false} />);
    const step = Math.ceil(text.length / chunks);
    for (let i = 1; i <= chunks; i++) {
        setContent(text.slice(0, Math.min(i * step, text.length)));
    }
    return result;
}

const SAMPLE = Array.from(
    { length: 40 },
    (_, i) => `## Section ${i}\n\nSome prose for section ${i} with \`inline\` code.\n\n`,
).join("");

describe("Markdown streaming-render work invariants", () => {
    it("builds the unified processor once, not once per commit", () => {
        __resetMarkdownRenderStats();
        streamInto(SAMPLE, 25);

        // The processor depends only on `highlight` and the slug prefix —
        // never on the text — so streaming must not rebuild it. It used to be
        // constructed inside the render memo, making this equal to the commit
        // count and costing ~4ms every commit.
        expect(__markdownRenderStats.commits).toBeGreaterThan(1);
        expect(__markdownRenderStats.processorBuilds).toBe(1);
    });

    /**
     * The way MarkdownBlock actually drives this component: `highlight` is
     * `view().highlight`, a getter over a signal that yields a NEW object on
     * every streaming commit. The processor memo tracked that signal through
     * the prop getter, so it re-ran (and rebuilt the whole plugin chain) once
     * per commit even though the boolean it derives never changed — live
     * counters read `processorBuilds == commits` under real streaming while
     * the static-prop test above stayed green. The memo must key off the
     * resolved boolean, not the getter's upstream signal.
     */
    it("does not rebuild the processor when `highlight` is a reactive getter that re-yields the same value", () => {
        __resetMarkdownRenderStats();
        const [view, setView] = createSignal({ text: "", highlight: false });
        render(() => <Markdown text={view().text} highlight={view().highlight} scrollable={false} />);
        const step = Math.ceil(SAMPLE.length / 20);
        for (let i = 1; i <= 20; i++) {
            setView({ text: SAMPLE.slice(0, Math.min(i * step, SAMPLE.length)), highlight: false });
        }

        expect(__markdownRenderStats.commits).toBeGreaterThan(10);
        expect(
            __markdownRenderStats.processorBuilds,
            `processor was built ${__markdownRenderStats.processorBuilds} times over ` +
                `${__markdownRenderStats.commits} commits — it must not track the text signal`,
        ).toBe(1);

        // A genuine flip still rebuilds it — exactly once.
        setView({ text: SAMPLE, highlight: true });
        expect(__markdownRenderStats.processorBuilds).toBe(2);
    });

    /**
     * `streaming` is the prop MarkdownBlock drives per commit now. Across a
     * stream that keeps flipping it (mid-stream pauses read as "settled"),
     * only two processors may ever exist — the final one and the plain one
     * for the open tail — and a flip must not re-parse the frozen prefix.
     */
    it("flipping `streaming` builds at most two processors and re-parses only the tail", () => {
        const run = (flip: boolean): number => {
            __resetMarkdownRenderStats();
            const [view, setView] = createSignal({ text: "", streaming: true });
            const r = render(() => <Markdown text={view().text} streaming={view().streaming} scrollable={false} />);
            const step = Math.ceil(SAMPLE.length / 20);
            for (let i = 1; i <= 20; i++) {
                const text = SAMPLE.slice(0, Math.min(i * step, SAMPLE.length));
                setView({ text, streaming: true });
                if (flip) setView({ text, streaming: false }); // a pause: settled render of the same text
            }
            r.unmount();
            return __markdownRenderStats.parsedChars;
        };
        const steady = run(false);
        const flipping = run(true);

        expect(__markdownRenderStats.processorBuilds).toBe(2);
        // A flip may re-parse the open tail — once more per commit, at most
        // doubling the work. Re-parsing the frozen prefix makes it scale with
        // the message instead (MarkdownBlock.stream-pauses.test.tsx measured
        // 11.4x the message before this was fixed).
        expect(
            flipping / steady,
            `flipping \`streaming\` every commit parsed ${flipping} chars vs ${steady} without — ` +
                `a flip re-parsed more than the open tail`,
        ).toBeLessThanOrEqual(2);
    });

    /**
     * Codex P2 on #3559. A completed `@@@start … @@@end` block renders as a
     * `<waveblock>` placeholder whose markdown text is identical no matter
     * what the block body holds, and MuxBlock reads its `blockmap` once at
     * creation. Once that placeholder is inside the frozen prefix, editing
     * ONLY the block body must still update it — the frozen DOM may not
     * outlive the block data it was rendered from.
     */
    it("re-renders a frozen content-block placeholder when only its block body changes", () => {
        const doc = (body: string) =>
            `${LONG_FILLER}@@@start file "notes.txt"\n${body}\n@@@end file "notes.txt"\n\n${LONG_FILLER}## Tail\n\nstill streaming`;
        const [content, setContent] = createSignal(doc("x".repeat(100)));
        const { container } = render(() => <Markdown text={content()} scrollable={false} />);
        expect(container.querySelector(".wave-block-size")?.textContent).toBe("0.1 KB");

        setContent(doc("x".repeat(5 * 1024)));
        expect(container.querySelector(".wave-block-size")?.textContent).toBe("5 KB");
    });

    it("does not rebuild the processor when only the text changes", () => {
        __resetMarkdownRenderStats();
        const [content, setContent] = createSignal("first");
        render(() => <Markdown text={content()} scrollable={false} />);
        const afterMount = __markdownRenderStats.processorBuilds;

        setContent("second, quite different text");
        setContent("third, different again");

        expect(__markdownRenderStats.processorBuilds).toBe(afterMount);
    });

    /**
     * THE load-bearing invariant: a streaming commit may only re-parse the
     * trailing open block, never the whole message.
     *
     * Before incremental parsing this was ~13x the message length at 25
     * commits, and grew with commit count — the O(n²) that burned 1.7s of main
     * thread on a 32KB response. If this assertion starts failing, something
     * has reverted to a full re-parse per commit; fix that rather than raising
     * the bound.
     */
    it("only re-parses the trailing block, not the whole message", () => {
        __resetMarkdownRenderStats();
        streamInto(SAMPLE, 25);

        const ratio = __markdownRenderStats.parsedChars / SAMPLE.length;
        expect(
            ratio,
            `parsed ${__markdownRenderStats.parsedChars} chars to render a ${SAMPLE.length}-char ` +
                `message over ${__markdownRenderStats.commits} commits (${ratio.toFixed(1)}x). ` +
                `Incremental parsing should keep this near 1x; a full re-parse per commit is ~13x.`,
        ).toBeLessThan(2);
    });

    /**
     * The DOM half of incremental rendering
     * (ANALYSIS_AGENT_PANE_FLUSH_REMOUNT_CHURN_2026_09_23.md §6.1).
     *
     * #3521 made the PARSE incremental but the render still handed
     * `toJsxRuntime` the whole frozen+tail hast every commit and swapped the
     * entire element tree, so every paragraph, heading, code block and table
     * of the streaming message was destroyed and re-created ~11×/s — measured
     * as ~950 elements re-created per 10 s in a pane with zero rows added.
     * The frozen prefix's DOM must survive commits; only the trailing open
     * block may be rebuilt.
     */
    it("keeps the frozen prefix's DOM nodes across commits — only the trailing block is rebuilt", () => {
        __resetMarkdownRenderStats();
        const [content, setContent] = createSignal("");
        const { container } = render(() => <Markdown text={content()} scrollable={false} />);
        const step = Math.ceil(SAMPLE.length / 25);

        // Stream far enough that the first heading is inside the frozen prefix.
        setContent(SAMPLE.slice(0, step * 12));
        const firstHeading = container.querySelector(".heading");
        expect(firstHeading).not.toBeNull();
        const headingCountBefore = container.querySelectorAll(".heading").length;

        for (let i = 13; i <= 25; i++) {
            setContent(SAMPLE.slice(0, Math.min(i * step, SAMPLE.length)));
        }

        // Same element object, still attached, and the document grew rather
        // than being rebuilt from scratch.
        expect(container.querySelector(".heading")).toBe(firstHeading);
        expect(container.contains(firstHeading)).toBe(true);
        expect(container.querySelectorAll(".heading").length).toBeGreaterThan(headingCountBefore);
    });

    /**
     * Regression, ReAgent P1 round 2 on PR #3521.
     *
     * Heading ids must be unique across the WHOLE document, not per parsed
     * segment. rehype-slug calls `slugs.reset()` on every transform run
     * (node_modules/rehype-slug/lib/index.js), so once a commit parses the
     * frozen prefix and the tail as two separate `runSync` calls, each gets
     * its own dedup namespace. Two headings with the same text landing on
     * opposite sides of the split would then both get `id="overview"` instead
     * of `overview` / `overview-1`, producing duplicate DOM ids — and
     * `getElementById` always resolves to the first, so TOC navigation to the
     * second heading silently scrolls to the wrong place.
     *
     * Asserts the user-visible contract (unique ids in the DOM) rather than
     * the mechanism, so it stays valid whichever way the split is made safe.
     */
    it("keeps heading ids unique across a split boundary", () => {
        // Streamed, not rendered in one shot: the two headings have to land in
        // DIFFERENT parsed segments for the per-segment slugger to collide. A
        // single render puts the whole document in one segment and hides it.
        const text = `## Overview\n\n${LONG_FILLER}## Overview\n\n${LONG_FILLER}`;
        const { container } = streamInto(text, 20);

        const ids = [...container.querySelectorAll("[id]")].map((el) => el.id);
        expect(ids.length, "expected heading ids to exist at all").toBeGreaterThan(1);

        const duplicates = ids.filter((id, i) => ids.indexOf(id) !== i);
        expect(duplicates, `duplicate DOM ids: ${JSON.stringify(duplicates)}`).toEqual([]);
    });

    /**
     * Scaling, not a fixed budget: doubling the commit count must not
     * meaningfully increase total work, because each byte is still parsed once.
     * This is the machine-independent way to assert "not O(n²)" — it compares
     * two counts from the same run rather than against any wall-clock number.
     */
    it("total parse work does not grow with commit count", () => {
        __resetMarkdownRenderStats();
        streamInto(SAMPLE, 10);
        const coarse = __markdownRenderStats.parsedChars;

        __resetMarkdownRenderStats();
        streamInto(SAMPLE, 40);
        const fine = __markdownRenderStats.parsedChars;

        expect(
            fine / coarse,
            `streaming the same message in 4x as many commits parsed ${fine} chars vs ${coarse} ` +
                `(${(fine / coarse).toFixed(1)}x). Under a full re-parse this scales with commit ` +
                `count; under incremental parsing it should stay flat.`,
        ).toBeLessThan(2);
    });
});

/**
 * The open tail is updated in place while only its text grows
 * (TRACKING_AGENT_PANE_BOUNDED_LIVE_WINDOW_2026_09_23.md §4: "the growing last
 * message's DOM is replaced on every commit" — ~23 % of streaming work in
 * `replaceChild`).
 *
 * The frozen prefix already survives commits; the tail — everything after the
 * last provably-safe split, which is the WHOLE block while a code fence, list
 * or table is still open — used to be re-rendered and swapped on every commit.
 * When a commit changes nothing but the tail's last run of text (the common
 * case: a code fence or paragraph growing), the tail's DOM must be kept and
 * that one text node updated. Anything structural still rebuilds, and the
 * result must always match a from-scratch render of the same text.
 */
describe("Markdown open-tail in-place updates", () => {
    /**
     * Rendered HTML for comparison across instances: per-instance slug prefixes
     * removed, and whitespace-only text between tags collapsed — segmented
     * rendering (frozen prefix + tail) drops the newline text node that a
     * single parse keeps between the two blocks at the seam. Invisible, and
     * pre-existing; not what these tests are about.
     */
    const html = (el: Element) => el.innerHTML.replace(/\sid="[^"]*"/g, "").replace(/>\s+</g, "><");

    /** Streams cumulative `texts` into one instance; also renders the final text from scratch. */
    function streamAndCompare(texts: string[]) {
        const [view, setView] = createSignal({ text: "", streaming: true });
        const live = render(() => <Markdown text={view().text} streaming={view().streaming} scrollable={false} />);
        for (const t of texts) setView({ text: t, streaming: true });
        const final = texts[texts.length - 1];
        const fresh = render(() => <Markdown text={final} streaming={true} scrollable={false} />);
        return { live, fresh };
    }

    it("keeps a growing code fence's DOM and updates its text in place", () => {
        __resetMarkdownRenderStats();
        const lines = Array.from({ length: 30 }, (_, i) => `const line${i} = ${i};`);
        const doc = (n: number) => `${LONG_FILLER}\`\`\`ts\n${lines.slice(0, n).join("\n")}\n`;

        const [view, setView] = createSignal({ text: doc(1), streaming: true });
        const { container } = render(() => <Markdown text={view().text} streaming={view().streaming} scrollable={false} />);
        const pre = container.querySelector("pre.codeblock");
        expect(pre).not.toBeNull();

        for (let n = 2; n <= 30; n++) setView({ text: doc(n), streaming: true });

        expect(container.querySelector("pre.codeblock"), "the open code block must not be rebuilt per commit").toBe(pre);
        expect(container.querySelector("pre.codeblock code")!.textContent).toContain("const line29 = 29;");
        expect(__markdownRenderStats.tailInPlaceUpdates).toBeGreaterThanOrEqual(28);
    });

    it("keeps a growing paragraph's DOM", () => {
        __resetMarkdownRenderStats();
        const words = Array.from({ length: 40 }, (_, i) => `word${i}`);
        const doc = (n: number) => `${LONG_FILLER}${words.slice(0, n).join(" ")}`;
        const [view, setView] = createSignal({ text: doc(1), streaming: true });
        const { container } = render(() => <Markdown text={view().text} streaming={view().streaming} scrollable={false} />);
        const paras = () => container.querySelectorAll(".paragraph");
        const last = paras()[paras().length - 1];

        for (let n = 2; n <= 40; n++) setView({ text: doc(n), streaming: true });

        expect(paras()[paras().length - 1]).toBe(last);
        expect(last.textContent).toBe(words.join(" "));
        expect(__markdownRenderStats.tailInPlaceUpdates).toBeGreaterThan(30);
    });

    it("matches a from-scratch render after in-place streaming (code fence, paragraph, list, table)", () => {
        const code = Array.from({ length: 12 }, (_, i) => `x${i}()`);
        const texts = code.map((_, i) => `${LONG_FILLER}\`\`\`py\n${code.slice(0, i + 1).join("\n")}\n`);
        const add = (s: string) => texts.push(texts[texts.length - 1] + s);
        add("```\n\nClosing paragraph that grows");
        add(" a little more");
        add("\n\n- item one\n- item two");
        add(" grows\n- item three");
        add("\n\n| a | b |\n|---|---|\n| 1 | 2 |");
        add("3");
        const { live, fresh } = streamAndCompare(texts);
        expect(html(live.container)).toBe(html(fresh.container));
    });

    it("rebuilds when inline structure appears, and still matches a fresh render", () => {
        __resetMarkdownRenderStats();
        const base = `${LONG_FILLER}Some text with **bol`;
        const { live, fresh } = streamAndCompare([base, `${base}d`, `${base}d** and more`]);
        expect(live.container.querySelector("strong")?.textContent).toBe("bold");
        expect(html(live.container)).toBe(html(fresh.container));
        expect(__markdownRenderStats.tailRebuilds).toBeGreaterThan(0);
    });

    it("never patches heading text in place — its slug id and TOC entry follow the text", () => {
        const texts = [`${LONG_FILLER}## Over`, `${LONG_FILLER}## Overvi`, `${LONG_FILLER}## Overview`];
        const { live, fresh } = streamAndCompare(texts);
        const headings = live.container.querySelectorAll(".heading");
        const last = headings[headings.length - 1] as HTMLElement;
        expect(last.textContent).toBe("Overview");
        expect(last.id.endsWith("overview")).toBe(true);
        expect(html(live.container)).toBe(html(fresh.container));
    });
});
