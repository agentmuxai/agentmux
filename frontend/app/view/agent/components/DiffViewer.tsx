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

import { createEffect, createSignal, onCleanup, Show, type JSX } from "solid-js";
import { For } from "solid-js";
import type { EditParams, EditResult } from "../types";
import { detectLanguage } from "./detectLanguage";

const ShikiTheme = "github-dark-high-contrast";

let shikiModule: typeof import("shiki/bundle/web") | null = null;
const getShiki = async () => {
    if (!shikiModule) shikiModule = await import("shiki/bundle/web");
    return shikiModule;
};

const CAP_LINES = 1500;

// Cache keyed on (filePath, diffText) since diffs are immutable once rendered.
const diffCache = new Map<string, string>();

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
}

export const DiffViewer = (props: DiffViewerProps): JSX.Element => {
    const diff = () => props.result?.diff;
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

    let preEl: HTMLPreElement | undefined;
    createEffect(() => {
        const h = highlightedHtml();
        if (preEl && h !== null) preEl.innerHTML = h;
    });

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

    // --- Plain fallback (shown until Shiki resolves, or permanently if Shiki fails) ---
    const PlainDiff = () => {
        const lines = parseDiffLines(diff()!);
        return (
            <pre class="agent-diff">
                <div class="agent-diff-header">{filePath()}</div>
                <For each={lines}>
                    {(line) => (
                        <div class={`agent-diff-${line.type}`}>{line.raw}</div>
                    )}
                </For>
            </pre>
        );
    };

    return (
        <Show
            when={highlightedHtml() !== null}
            fallback={<PlainDiff />}
        >
            {/* Header lives outside the <pre> so that innerHTML = shikiHtml
                does not overwrite it. The <pre> receives only Shiki content. */}
            <div class="agent-diff agent-diff--highlighted">
                <div class="agent-diff-header">{filePath()}</div>
                <pre ref={preEl} class="agent-diff-highlighted-body" />
            </div>
        </Show>
    );
};

DiffViewer.displayName = "DiffViewer";
