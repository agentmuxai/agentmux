// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DiffViewer — unified-diff display with token-level syntax highlighting.
 *
 * Step 3 of SPEC_TOOL_OVERLAY_CODE_HIGHLIGHTING_2026_04_14.md.
 *
 * Rendering strategy:
 *  1. Parse unified diff lines, classify each as add/del/hunk/ctx.
 *  2. Reconstruct the "clean" source (stripped of +/-/space prefix) and pass
 *     it through Shiki with a custom line transformer that re-injects the
 *     diff CSS class on the Shiki-generated <span class="line"> element.
 *  3. Inject the resulting HTML via innerHTML on the <pre> wrapper.
 *  4. Falls back to the plain line-by-line render if Shiki fails or the
 *     diff is too large.
 *
 * The `.agent-diff-add` / `-del` / `-hunk` / `-ctx` SCSS rules remain
 * unchanged — they now target `.line.agent-diff-add` etc. in the Shiki
 * output.
 */

import { createEffect, createMemo, createSignal, Index, onCleanup, Show, type JSX } from "solid-js";
import type { EditParams, EditResult } from "../types";
import { detectLanguage } from "./detectLanguage";
import { OutputHiddenMarker } from "./OutputHiddenMarker";
import { capText, MAX_TOOL_OUTPUT_LINES } from "./output-cap";
import { formatDiffSides } from "./dedent";

const ShikiTheme = "github-dark-high-contrast";

let shikiModule: typeof import("shiki/bundle/web") | null = null;
const getShiki = async () => {
    if (!shikiModule) shikiModule = await import("shiki/bundle/web");
    return shikiModule;
};

const CAP_LINES = 1500;

// Cache keyed on (filePath, diffText) since diffs are immutable once rendered.
const diffCache = new Map<string, string>();

// LCS line-diff used when result.diff is absent (the common case — Claude's
// translator returns a plain-text string, not a structured EditResult, so
// result.diff is always undefined in the current pipeline).
const LCS_LINE_LIMIT = 300;

function lcsUnifiedDiff(oldLines: string[], newLines: string[]): string {
    const m = oldLines.length;
    const n = newLines.length;
    // Uint16Array: max LCS score 65535; safe for ≤ LCS_LINE_LIMIT lines.
    const dp = Array.from({ length: m + 1 }, () => new Uint16Array(n + 1));
    for (let i = 1; i <= m; i++) {
        for (let j = 1; j <= n; j++) {
            dp[i][j] = oldLines[i - 1] === newLines[j - 1]
                ? dp[i - 1][j - 1] + 1
                : Math.max(dp[i - 1][j], dp[i][j - 1]);
        }
    }
    const edits: Array<[string, string]> = [];
    let i = m, j = n;
    while (i > 0 || j > 0) {
        if (i > 0 && j > 0 && oldLines[i - 1] === newLines[j - 1]) {
            edits.unshift([" ", oldLines[i - 1]]);
            i--; j--;
        } else if (j > 0 && (i === 0 || dp[i][j - 1] >= dp[i - 1][j])) {
            edits.unshift(["+", newLines[j - 1]]);
            j--;
        } else {
            edits.unshift(["-", oldLines[i - 1]]);
            i--;
        }
    }
    let oldCount = 0, newCount = 0;
    for (const [op] of edits) {
        if (op !== "+") oldCount++;
        if (op !== "-") newCount++;
    }
    const out = [`@@ -1,${oldCount} +1,${newCount} @@`];
    for (const [op, line] of edits) out.push(op + line);
    return out.join("\n");
}

function buildDiffFromParams(oldStr: string, newStr: string): string {
    const oldLines = oldStr.split("\n");
    const newLines = newStr.split("\n");
    if (oldLines.length <= LCS_LINE_LIMIT && newLines.length <= LCS_LINE_LIMIT) {
        return lcsUnifiedDiff(oldLines, newLines);
    }
    // Large edit: plain del+add block — no LCS cost, still legible.
    return [
        `@@ -1,${oldLines.length} +1,${newLines.length} @@`,
        ...oldLines.map(l => "-" + l),
        ...newLines.map(l => "+" + l),
    ].join("\n");
}

type LineType = "add" | "del" | "hunk" | "ctx";

interface DiffLine {
    type: LineType;
    raw: string;   // original line including +/-/space prefix
    body: string;  // line without the leading marker
}

function parseDiffLines(diff: string): DiffLine[] {
    return diff.split("\n").map((raw) => {
        if (raw.startsWith("+")) return { type: "add" as LineType, raw, body: raw.slice(1) };
        if (raw.startsWith("-")) return { type: "del" as LineType, raw, body: raw.slice(1) };
        if (raw.startsWith("@")) return { type: "hunk" as LineType, raw, body: raw };
        return { type: "ctx" as LineType, raw, body: raw.startsWith(" ") ? raw.slice(1) : raw };
    });
}

interface DiffViewerProps {
    params: EditParams;
    result?: EditResult;
    /** ToolNode.status — gates the params fallback. Only synthesise a diff
     *  when the edit actually applied. For failed/denied edits the params
     *  reflect the *intended* change, not what happened, so showing them
     *  as a diff would mislead. Defaults to "success" when omitted. */
    status?: string;
}

export const DiffViewer = (props: DiffViewerProps): JSX.Element => {
    const rawDiff = () => {
        // Prefer the pre-computed diff from result (populated by some providers).
        if (props.result?.diff) return props.result.diff;
        // Fall back to computing from params only for successful edits.
        // For failed/denied nodes params reflect the *intended* change —
        // synthesising a diff would show what was supposed to happen, not
        // what did, misleading the user. (Codex P2 on PR #1561.)
        const succeeded = (props.status ?? "success") === "success";
        if (succeeded) {
            const p = props.params;
            if (p && (p.old_string != null || p.new_string != null)) {
                // Dedent AND narrow BEFORE building the diff, with one prefix
                // and one indent unit shared across both sides
                // (SPEC_TOOL_PREVIEW_DEDENT_2026_08_08.md §3.2.2) — treating
                // the sides independently could shift one by a different
                // amount than the other (e.g. new_string adding a shallower
                // wrapper line) and manufacture a phantom indentation diff
                // that was never actually part of the edit. It has to happen
                // here rather than on the built diff: once the +/- markers are
                // on, the "leading whitespace" of a line is the marker.
                const { oldStr, newStr } = formatDiffSides(p.old_string ?? "", p.new_string ?? "");
                return buildDiffFromParams(oldStr, newStr);
            }
        }
        return undefined;
    };
    // Head-cap the diff (read top-down) so the plain fallback (one <div> per
    // line, used for diffs > CAP_LINES) can't bloat the conversation DOM.
    // Memoized: this is read several times per render (highlight effect,
    // PlainDiff, hidden-lines marker) and capText splits the whole diff, so a
    // bare accessor would re-split a large diff several times each frame.
    const diffCap = createMemo(() => {
        const d = rawDiff();
        return d ? capText(d, MAX_TOOL_OUTPUT_LINES, "head") : null;
    });
    const diff = () => diffCap()?.text;
    const filePath = () => props.params.file_path ?? "";

    // --- Highlighted path ---
    const [highlightedHtml, setHighlightedHtml] = createSignal<string | null>(null);
    let seq = 0;

    createEffect(() => {
        const d = diff();
        const fp = filePath();
        setHighlightedHtml(null);
        if (!d) return;

        const lines = parseDiffLines(d);
        if (lines.length > CAP_LINES) return; // fall back to plain render

        const key = `${fp}::${d}`;
        if (diffCache.has(key)) {
            setHighlightedHtml(diffCache.get(key)!);
            return;
        }

        const mySeq = ++seq;
        let cancelled = false;
        onCleanup(() => { cancelled = true; });

        void (async () => {
            try {
                const { codeToHtml } = await getShiki();
                if (cancelled || mySeq !== seq) return;

                const lang = detectLanguage(fp);

                // Build clean source (strip diff markers so Shiki sees valid code)
                const cleanSource = lines.map((l) => l.body).join("\n");

                // Shiki transformer: after each line is wrapped in <span class="line">,
                // inject the diff CSS class based on the original line type.
                const lineTypes = lines.map((l) => l.type);
                let lineIdx = 0;
                const transformer = {
                    line(node: any) {
                        const type = lineTypes[lineIdx++] ?? "ctx";
                        const existing = (node.properties?.class ?? "") as string;
                        node.properties = node.properties ?? {};
                        node.properties.class = `${existing} agent-diff-${type}`.trim();
                        // Re-insert the marker character as a non-highlighted prefix span
                        if (type === "add" || type === "del") {
                            const marker = type === "add" ? "+" : "-";
                            node.children = [
                                { type: "element", tagName: "span",
                                  properties: { class: "agent-diff-marker" },
                                  children: [{ type: "text", value: marker }] },
                                ...node.children,
                            ];
                        } else if (type === "hunk") {
                            // Hunk headers: re-render as plain text (grammar doesn't know @@ syntax)
                            node.children = [
                                { type: "element", tagName: "span",
                                  properties: { class: "agent-diff-hunk-text" },
                                  children: [{ type: "text", value: lines[lineIdx - 1]?.raw ?? "" }] },
                            ];
                        } else {
                            // ctx: space prefix
                            node.children = [
                                { type: "element", tagName: "span",
                                  properties: { class: "agent-diff-marker" },
                                  children: [{ type: "text", value: " " }] },
                                ...node.children,
                            ];
                        }
                    },
                };

                const full = await codeToHtml(cleanSource, {
                    lang: lang === "text" ? "plaintext" : lang,
                    theme: ShikiTheme,
                    transformers: [transformer],
                });
                if (cancelled || mySeq !== seq) return;

                // Extract inner content of the <pre>
                const preStart = full.indexOf("<pre");
                const preOpen = full.indexOf(">", preStart);
                const preEnd = full.lastIndexOf("</pre>");
                const inner = preStart !== -1 && preOpen !== -1 && preEnd !== -1
                    ? full.slice(preOpen + 1, preEnd)
                    : full;

                diffCache.set(key, inner);
                setHighlightedHtml(inner);
            } catch (e) {
                // Fall through to plain render
                console.warn("[DiffViewer] Shiki highlight failed", e);
            }
        })();
    });

    // Applying the Shiki HTML has to happen BOTH when the html arrives and when
    // the element is created, because neither event implies the other.
    //
    // The <pre> lives inside the `highlightedHtml() !== null` <Show> below, so
    // the same signal that carries the html also creates the element that
    // receives it. An effect alone loses that race: it can run while `preEl` is
    // still undefined, the `if (preEl && ...)` guard silently does nothing, and
    // because the signal never changes again nothing ever fills the freshly
    // created <pre>. What the user sees is the diff appearing (the plain
    // fallback, while Shiki loads) and then vanishing the moment highlighting
    // resolves, leaving just the file-path header — reported as "the preview
    // shows for a moment, then is replaced by a single path to the file", and
    // confirmed live: `.agent-diff--highlighted` mounted with its <pre> at
    // height 0 and textContent "".
    //
    // `applyHighlight` is therefore called from the ref callback too, which
    // runs exactly when the element exists. Whichever happens last wins, and
    // both orderings end up with a filled <pre>. (`HighlightedCode` never hit
    // this because its <pre> is unconditionally mounted.)
    let preEl: HTMLPreElement | undefined;
    const applyHighlight = () => {
        const h = highlightedHtml();
        if (preEl && h !== null) preEl.innerHTML = h;
    };
    createEffect(applyHighlight);

    // --- No diff path ---
    if (!diff()) {
        return (
            <pre class="agent-diff-empty">
                No diff available
                {"\n"}
                File: {filePath()}
            </pre>
        );
    }

    // Memoized so parseDiffLines doesn't re-run on every reactive sweep
    // and so <Index> sees a stable array reference between renders.
    const lines = createMemo(() => {
        const d = diff();
        return d ? parseDiffLines(d) : [];
    });

    // --- Plain fallback (shown until Shiki resolves, or permanently if Shiki fails) ---
    // Uses <Index> (position-keyed) rather than <For> (reference-keyed).
    // Diffs are immutable once rendered, but a <Show> branch flip when
    // Shiki resolves can race with a reactive update that drives a
    // second reconcileArrays pass on the same DOM nodes → replaceChild
    // NotFoundError. <Index> avoids that: each position slot updates
    // in place; no DOM moves occur when the array is stable. (#1326)
    const PlainDiff = () => (
        <pre class="agent-diff">
            <div class="agent-diff-header">{filePath()}</div>
            <Index each={lines()}>
                {(line) => (
                    <div class={`agent-diff-${line().type}`}>{line().raw}</div>
                )}
            </Index>
        </pre>
    );

    return (
        <>
            <Show
                when={highlightedHtml() !== null}
                fallback={<PlainDiff />}
            >
                {/* Header lives outside the <pre> so that innerHTML = shikiHtml
                    does not overwrite it. The <pre> receives only Shiki content. */}
                <div class="agent-diff agent-diff--highlighted">
                    <div class="agent-diff-header">{filePath()}</div>
                    <pre
                        ref={(el) => {
                            preEl = el;
                            applyHighlight();
                        }}
                        class="agent-diff-highlighted-body"
                    />
                </div>
            </Show>
            <Show when={(diffCap()?.hiddenLines ?? 0) > 0}>
                <OutputHiddenMarker hidden={diffCap()!.hiddenLines} noun="line" from="head" />
            </Show>
        </>
    );
};

DiffViewer.displayName = "DiffViewer";
