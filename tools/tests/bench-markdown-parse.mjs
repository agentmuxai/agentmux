#!/usr/bin/env npx tsx
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// bench-markdown-parse — how expensive is ONE agent-pane streaming commit,
// and does that cost grow with message length?
//
// WHY THIS EXISTS
//
// `MarkdownBlock.tsx`'s own comment says re-parsing a whole streaming message
// every frame is O(n^2) and starves keystrokes. The shipped mitigation (#1213)
// throttles commits to STREAM_RENDER_MS (90ms) and skips highlighting
// mid-stream. That lowers the *rate* of the expensive operation; it does not
// change its *cost*, which is still O(n) in the whole message on every commit.
//
// Whether that actually matters is an empirical question this answers:
//   - if per-commit cost is FLAT in n, the O(n^2) framing is irrelevant in
//     practice and ANALYSIS_AGENT_PANE_TYPING_UNDER_LOAD_2026_09_22.md §2 is
//     refuted — do not rewrite the markdown path;
//   - if it RISES and crosses a frame budget at realistic message sizes, §2 is
//     confirmed and incremental parsing is the fix.
//
// Deliberately measures the REAL pipeline from `markdown.tsx:271-286` — same
// plugins, same order, including the two app-local remark plugins — not a
// lookalike. What it does NOT cover is `toJsxRuntime` + Solid's DOM reconcile,
// which run after this and cost MORE on top. So every number here is a LOWER
// BOUND on a real commit.
//
// USAGE
//   npx tsx tools/tests/bench-markdown-parse.mjs
//   npx tsx tools/tests/bench-markdown-parse.mjs --file <path-to-real-output.md>
//   npx tsx tools/tests/bench-markdown-parse.mjs --stream 64          # simulate one 64KB message streaming
//   npx tsx tools/tests/bench-markdown-parse.mjs --code-ratio 0.5     # heavier code content
//
// Must run under `tsx` (it imports the app's TypeScript plugins directly).

import { readFileSync } from "node:fs";
import { unified } from "unified";
import remarkParse from "remark-parse";
import remarkGfm from "remark-gfm";
import remarkRehype from "remark-rehype";
import rehypeRaw from "rehype-raw";
import rehypeHighlight from "rehype-highlight";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import rehypeSlug from "rehype-slug";
import RemarkFlexibleToc from "remark-flexible-toc";

import remarkMermaidToTag from "../../frontend/app/element/remark-mermaid-to-tag";
import { createContentBlockPlugin } from "../../frontend/app/element/markdown-contentblock-plugin";
import { findSafeSplitPoint } from "../../frontend/app/element/markdown-incremental";

// STREAM_RENDER_MS from MarkdownBlock.tsx:30 — the commit interval this is
// simulating. Kept as a literal (not imported) so the bench does not drag the
// whole Solid component graph in; if that constant changes, change this.
const STREAM_RENDER_MS = 90;
const FRAME_BUDGET_MS = 16.7; // one 60Hz frame

const args = process.argv.slice(2);
const flag = (name, fallback) => {
    const i = args.indexOf(`--${name}`);
    return i >= 0 && args[i + 1] != null ? args[i + 1] : fallback;
};
const has = (name) => args.includes(`--${name}`);

const codeRatio = Number(flag("code-ratio", "0.25"));
const streamKb = flag("stream", null);
const corpusFile = flag("file", null);

// ---------------------------------------------------------------------------
// Corpus: realistic agent output. Markdown cost is content-shaped — code blocks
// (especially with highlighting) dominate prose — so a corpus of pure lorem
// would understate the real cost. Deterministic, so runs are comparable.
// ---------------------------------------------------------------------------
function mulberry32(seed) {
    return () => {
        seed |= 0;
        seed = (seed + 0x6d2b79f5) | 0;
        let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
        t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
}

const PROSE = [
    "The reducer now owns this transition, so the pane no longer needs to reconcile it on mount.",
    "That path was already covered by the existing guard, but the guard ran after the dispatch.",
    "Worth noting: this only reproduces when more than one pane is streaming at the same time.",
    "I checked the call sites and all three go through the same choke point, so one fix covers them.",
    "The tricky part is that the symptom looks identical to the earlier bug that was already fixed.",
];
const CODE = [
    `function flushBatch(items) {\n    const out = [];\n    for (const item of items) {\n        if (!item.ready) continue;\n        out.push(transform(item));\n    }\n    return out;\n}`,
    `const handler = async (req, res) => {\n    const body = await parse(req);\n    if (!body?.id) {\n        return res.status(400).json({ error: "missing id" });\n    }\n    return res.json(await lookup(body.id));\n};`,
    `impl Flusher {\n    fn drain(&mut self) -> Vec<Chunk> {\n        let mut batch = Vec::new();\n        while let Ok(c) = self.rx.try_recv() {\n            batch.push(c);\n        }\n        batch\n    }\n}`,
];

function buildCorpus(targetBytes, seed = 42) {
    const rand = mulberry32(seed);
    const parts = [];
    let size = 0;
    let h = 0;
    while (size < targetBytes) {
        const roll = rand();
        let chunk;
        if (roll < codeRatio) {
            const lang = ["ts", "js", "rust", "python"][Math.floor(rand() * 4)];
            chunk = "```" + lang + "\n" + CODE[Math.floor(rand() * CODE.length)] + "\n```\n\n";
        } else if (roll < codeRatio + 0.08) {
            chunk = `## Section ${++h}\n\n`;
        } else if (roll < codeRatio + 0.2) {
            chunk = `- ${PROSE[Math.floor(rand() * PROSE.length)]}\n- ${PROSE[Math.floor(rand() * PROSE.length)]}\n\n`;
        } else if (roll < codeRatio + 0.24) {
            chunk = `| field | value |\n|---|---|\n| id | 42 |\n| state | ready |\n\n`;
        } else {
            chunk = PROSE[Math.floor(rand() * PROSE.length)] + " `inline()` too.\n\n";
        }
        parts.push(chunk);
        size += chunk.length;
    }
    return parts.join("");
}

// ---------------------------------------------------------------------------
// The pipeline — mirrors markdown.tsx:230-286 exactly, including plugin order.
// ---------------------------------------------------------------------------
const ALIGN_CLASS_REGEX = /^align-(left|center|right)$/;

function buildProcessor(highlight) {
    const tocRef = [];
    const rehypePlugins = [
        rehypeRaw,
        ...(highlight ? [rehypeHighlight] : []),
        // rehypeAlignToClass / rehypeLinkify are app-local and trivial
        // (single unist visits); omitted here, so this is a slight
        // UNDER-estimate of the real cost, consistent with the rest.
        () =>
            rehypeSanitize({
                ...defaultSchema,
                attributes: {
                    ...defaultSchema.attributes,
                    span: [...(defaultSchema.attributes?.span || []), ["className", /^hljs-./], ["srcset"], ["media"], ["type"]],
                    th: [...(defaultSchema.attributes?.th || []), ["className", ALIGN_CLASS_REGEX]],
                    td: [...(defaultSchema.attributes?.td || []), ["className", ALIGN_CLASS_REGEX]],
                    waveblock: [["blockkey"]],
                },
                tagNames: [...(defaultSchema.tagNames || []), "span", "waveblock", "picture", "source", "mermaidblock"],
            }),
        () => rehypeSlug({ prefix: "bench-" }),
    ];
    const remarkPlugins = [
        remarkMermaidToTag,
        remarkGfm,
        [RemarkFlexibleToc, { tocRef }],
        [createContentBlockPlugin, { blocks: new Map() }],
    ];
    return unified()
        .use(remarkParse)
        .use(remarkPlugins)
        .use(remarkRehype, { allowDangerousHtml: true })
        .use(rehypePlugins);
}

/** One streaming commit: parse + run all plugins, exactly as the memo does. */
function commit(text, highlight) {
    const processor = buildProcessor(highlight);
    const mdast = processor.parse(text);
    processor.runSync(mdast);
}

function timeCommit(text, highlight, reps) {
    const samples = [];
    for (let i = 0; i < reps; i++) {
        const t0 = performance.now();
        commit(text, highlight);
        samples.push(performance.now() - t0);
    }
    samples.sort((a, b) => a - b);
    return {
        median: samples[Math.floor(samples.length / 2)],
        min: samples[0],
        max: samples[samples.length - 1],
    };
}

const fmt = (n, w = 8) => n.toFixed(2).padStart(w);

// ---------------------------------------------------------------------------
// Mode 1 (default): per-commit cost vs message length.
// ---------------------------------------------------------------------------
function runScalingTable() {
    const sizes = [1, 2, 4, 8, 16, 32, 64, 128].map((kb) => kb * 1024);
    const base = corpusFile ? readFileSync(corpusFile, "utf8") : buildCorpus(sizes[sizes.length - 1] * 1.2);

    console.log(`\nONE STREAMING COMMIT — parse + plugins (lower bound: excludes JSX + DOM reconcile)`);
    console.log(`corpus: ${corpusFile ?? `synthetic, code-ratio ${codeRatio}`}    frame budget: ${FRAME_BUDGET_MS}ms\n`);
    console.log(`     size   mid-stream(no hl)      ms/KB      settle(highlight)      ms/KB   over frame?`);
    console.log(`  ───────   ─────────────────   ────────   ────────────────────   ────────   ───────────`);

    const rows = [];
    for (const size of sizes) {
        if (size > base.length) continue;
        const text = base.slice(0, size);
        const reps = size <= 8 * 1024 ? 15 : size <= 32 * 1024 ? 8 : 4;
        const plain = timeCommit(text, false, reps);
        const hl = timeCommit(text, true, reps);
        rows.push({ size, plain: plain.median, hl: hl.median });
        const kb = size / 1024;
        const over = plain.median > FRAME_BUDGET_MS ? "YES (mid-stream)" : hl.median > FRAME_BUDGET_MS ? "settle only" : "no";
        console.log(
            `  ${String(kb).padStart(5)}KB   ${fmt(plain.median)}ms         ${fmt(plain.median / kb, 6)}   ${fmt(hl.median)}ms            ${fmt(hl.median / kb, 6)}   ${over}`,
        );
    }

    // Scaling exponent: fit log(t) = a + b*log(n). b≈1 linear, b≈2 quadratic.
    // Flat cost (b≈0) would refute the premise outright.
    const fit = (key) => {
        const pts = rows.filter((r) => r[key] > 0.05);
        if (pts.length < 3) return NaN;
        const xs = pts.map((r) => Math.log(r.size));
        const ys = pts.map((r) => Math.log(r[key]));
        const mx = xs.reduce((a, b) => a + b) / xs.length;
        const my = ys.reduce((a, b) => a + b) / ys.length;
        let num = 0, den = 0;
        for (let i = 0; i < xs.length; i++) {
            num += (xs[i] - mx) * (ys[i] - my);
            den += (xs[i] - mx) ** 2;
        }
        return num / den;
    };

    console.log(`\n  per-commit scaling exponent (cost ∝ n^b):`);
    console.log(`     mid-stream b = ${fit("plain").toFixed(2)}     settle b = ${fit("hl").toFixed(2)}`);
    console.log(`     b≈0 → flat, §2 REFUTED · b≈1 → linear per commit (⇒ O(n²) per message) · b>1 → worse`);
    return rows;
}

// ---------------------------------------------------------------------------
// Mode 2 (--stream): replay one message actually streaming, summing every
// commit — the total main-thread time the pane burns to render ONE response.
// ---------------------------------------------------------------------------
function runStreamSimulation(kb) {
    const total = kb * 1024;
    const base = corpusFile ? readFileSync(corpusFile, "utf8") : buildCorpus(total * 1.1);
    // Agent output arrives at roughly 40-80 chars per 90ms commit window at
    // typical token rates; 800 is deliberately generous (fewer, larger commits)
    // so this UNDER-counts rather than inflates.
    const bytesPerCommit = 800;
    const commits = Math.floor(total / bytesPerCommit);

    console.log(`\nSTREAM SIMULATION — one ${kb}KB message, ${commits} commits @ ${STREAM_RENDER_MS}ms`);
    console.log(`(mid-stream commits un-highlighted, one final highlighted settle)\n`);

    let sum = 0;
    let worst = 0;
    let overBudget = 0;
    for (let i = 1; i <= commits; i++) {
        const text = base.slice(0, i * bytesPerCommit);
        const t0 = performance.now();
        commit(text, false);
        const dt = performance.now() - t0;
        sum += dt;
        worst = Math.max(worst, dt);
        if (dt > FRAME_BUDGET_MS) overBudget++;
    }
    const t0 = performance.now();
    commit(base.slice(0, total), true);
    const settle = performance.now() - t0;

    const wall = (commits * STREAM_RENDER_MS) / 1000;
    console.log(`  total main-thread time in parse:   ${sum.toFixed(0)}ms`);
    console.log(`  final settle (highlighted):        ${settle.toFixed(0)}ms   <- single longest block`);
    console.log(`  worst mid-stream commit:           ${worst.toFixed(1)}ms`);
    console.log(`  commits over one frame:            ${overBudget}/${commits}`);
    console.log(`  stream wall-clock:                 ~${wall.toFixed(1)}s`);
    console.log(`  main thread occupied by parse:     ${((sum / (wall * 1000)) * 100).toFixed(1)}%`);
    console.log(`\n  A keystroke landing inside any one commit waits for it to finish.`);
    console.log(`  Occupancy is the rough probability a given keystroke hits one;`);
    console.log(`  the worst-commit figure is how long the worst case stalls.`);
    console.log(`\n  NOTE: this is ONE pane. Backgrounded agent tabs stay mounted under`);
    console.log(`  keep-alive (PR #3391) with no visibility gate, so N streaming panes`);
    console.log(`  put ~N x this on the same main thread.`);
}

// ---------------------------------------------------------------------------
// Mode 3 (--incremental): what the fix would buy.
//
// Models the core of incremental markdown: markdown is block-structured, so
// everything before the LAST BLANK LINE of the committed text is structurally
// final. Freeze it, and re-parse only the trailing incomplete block each
// commit. Per-commit cost becomes O(trailing block), not O(whole message).
//
// This measures the PARSE-COST CEILING of the fix, not its correctness. A real
// implementation also has to handle the DOM side (append, don't replace) and
// the constructs that can reach backwards across a blank line — reference-style
// link definitions, loose lists, setext headings. Those affect what is safe to
// freeze; they do not change the cost shape measured here.
// ---------------------------------------------------------------------------
function runIncrementalComparison(kb) {
    const total = kb * 1024;
    const base = corpusFile ? readFileSync(corpusFile, "utf8") : buildCorpus(total * 1.1);
    const bytesPerCommit = 800;
    const commits = Math.floor(total / bytesPerCommit);

    console.log(`\nCURRENT vs INCREMENTAL — one ${kb}KB message, ${commits} commits\n`);

    let curSum = 0;
    let curWorst = 0;
    for (let i = 1; i <= commits; i++) {
        const text = base.slice(0, i * bytesPerCommit);
        const t0 = performance.now();
        commit(text, false);
        const dt = performance.now() - t0;
        curSum += dt;
        curWorst = Math.max(curWorst, dt);
    }

    // Models what actually ships: the real `findSafeSplitPoint`, plus the
    // frozen-prefix accumulation from markdown.tsx — each newly-frozen segment
    // is parsed exactly once, and only the trailing open block is re-parsed
    // per commit.
    let incSum = 0;
    let incWorst = 0;
    let frozenEnd = 0;
    for (let i = 1; i <= commits; i++) {
        const text = base.slice(0, i * bytesPerCommit);
        const safe = findSafeSplitPoint(text);
        const t0 = performance.now();
        if (safe > frozenEnd) {
            commit(text.slice(frozenEnd, safe), false);
            frozenEnd = safe;
        }
        commit(text.slice(frozenEnd), false);
        const dt = performance.now() - t0;
        incSum += dt;
        incWorst = Math.max(incWorst, dt);
    }

    const wall = (commits * STREAM_RENDER_MS) / 1000;
    const pct = (ms) => ((ms / (wall * 1000)) * 100).toFixed(1);
    console.log(`                          current      incremental      change`);
    console.log(`  ──────────────────   ──────────   ──────────────   ─────────`);
    console.log(`  total parse time      ${curSum.toFixed(0).padStart(7)}ms   ${incSum.toFixed(0).padStart(11)}ms   ${(curSum / incSum).toFixed(1)}x faster`);
    console.log(`  worst commit          ${curWorst.toFixed(1).padStart(7)}ms   ${incWorst.toFixed(1).padStart(11)}ms   ${(curWorst / incWorst).toFixed(1)}x faster`);
    console.log(`  main-thread occupancy ${pct(curSum).padStart(6)}%    ${pct(incSum).padStart(11)}%`);
    console.log(`\n  Worst commit is the number that decides felt latency: it is the`);
    console.log(`  longest a keystroke can be stuck waiting. Frame budget ${FRAME_BUDGET_MS}ms.`);
}

console.log(`bench-markdown-parse — node ${process.version}`);
if (has("incremental")) {
    runIncrementalComparison(Number(flag("incremental", "32")));
} else if (streamKb) {
    runStreamSimulation(Number(streamKb));
} else {
    runScalingTable();
    console.log(`\n  (--stream 64 simulates one message streaming; --incremental 32 compares the fix)`);
}
