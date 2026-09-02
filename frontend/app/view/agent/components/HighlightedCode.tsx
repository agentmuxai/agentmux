// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * HighlightedCode — async Shiki syntax highlighting for tool overlay content.
 *
 * Renders a plain <pre> immediately (no layout shift) then swaps in the
 * Shiki-generated HTML once the highlight resolves. Uses the same lazy-load
 * pattern and theme as streamdown.tsx so the Shiki chunk is only fetched once
 * across the whole app.
 *
 * Features:
 *  - WeakMap-based per-node cache: re-hovering the same tool block is instant.
 *  - Size cap: files > CAP_BYTES or > CAP_LINES skip highlighting (avoids
 *    stalling the main thread on huge files).
 *  - Sequence guard: stale async results are discarded if props changed.
 *  - Error fallback: Shiki failures silently degrade to plaintext.
 *
 * Spec: docs/specs/SPEC_TOOL_OVERLAY_CODE_HIGHLIGHTING_2026_04_14.md §4.1
 */

import { createEffect, createSignal, onCleanup, type JSX } from "solid-js";

const ShikiTheme = "github-dark-high-contrast";

// Lazy-load shiki — same singleton as streamdown.tsx so the chunk is
// only fetched once per app session.
let shikiModule: typeof import("shiki/bundle/web") | null = null;
const getShiki = async () => {
    if (!shikiModule) {
        shikiModule = await import("shiki/bundle/web");
    }
    return shikiModule;
};

// Size caps — skip highlighting for very large content to avoid blocking
// the main thread.
const CAP_BYTES = 200 * 1024; // 200 KB
const CAP_LINES = 2000;

// Module-level cache: ToolNode objects are immutable, so the same
// (code, lang) pair always produces the same HTML. Key on the full code +
// lang string — no collision risk, and the code is already in memory.
const highlightCache = new Map<string, string>();

function cacheKey(code: string, lang: string): string {
    return `${lang}:${code}`;
}

interface HighlightedCodeProps {
    code: string;
    /** Shiki language id — "text" for plain. Falls back to plaintext on failure. */
    lang: string;
    /** Extra CSS class applied to the outer <pre>. */
    class?: string;
}

export const HighlightedCode = (props: HighlightedCodeProps): JSX.Element => {
    const [html, setHtml] = createSignal<string | null>(null);
    let seq = 0;
    let preEl: HTMLPreElement | undefined;

    createEffect(() => {
        const code = props.code;
        const lang = props.lang;

        // Reset to plaintext while the new highlight computes
        setHtml(null);

        // Trivial: plain text or empty
        if (!code || lang === "text") return;

        // Size cap
        if (code.length > CAP_BYTES || code.split("\n").length > CAP_LINES) {
            console.debug(`[HighlightedCode] skipping highlight: file too large (${code.length} bytes)`);
            return;
        }

        // Cache hit — apply synchronously
        const key = cacheKey(code, lang);
        if (highlightCache.has(key)) {
            setHtml(highlightCache.get(key)!);
            return;
        }

        // Async highlight
        const mySeq = ++seq;
        let cancelled = false;
        onCleanup(() => { cancelled = true; });

        void (async () => {
            try {
                const { codeToHtml } = await getShiki();
                if (cancelled || mySeq !== seq) return;
                const full = await codeToHtml(code, { lang, theme: ShikiTheme });
                if (cancelled || mySeq !== seq) return;
                // Shiki wraps output in <pre><code>...</code></pre>; we only
                // want the inner HTML of the <pre> so our own <pre> wrapper
                // gets the Shiki token spans without the outer element.
                const preStart = full.indexOf("<pre");
                const preOpen = full.indexOf(">", preStart);
                const preEnd = full.lastIndexOf("</pre>");
                const inner = preStart !== -1 && preOpen !== -1 && preEnd !== -1
                    ? full.slice(preOpen + 1, preEnd)
                    : full;
                highlightCache.set(key, inner);
                setHtml(inner);
            } catch (e) {
                if (!cancelled && mySeq === seq) {
                    // Stay on plaintext fallback; don't retry
                    console.warn(`[HighlightedCode] Shiki failed for lang=${lang}`, e);
                }
            }
        })();
    });

    // When html() is set, inject it via innerHTML. Shiki output is sanitised
    // (only <span> elements with class/style attributes), so this is safe.
    createEffect(() => {
        const h = html();
        if (preEl && h !== null) {
            preEl.innerHTML = h;
        } else if (preEl && h === null) {
            // Reset to text content (plaintext fallback while loading)
            preEl.textContent = props.code;
        }
    });

    return (
        <pre
            ref={preEl}
            class={`agent-highlighted-code${props.class ? ` ${props.class}` : ""}`}
        >
            {props.code}
        </pre>
    );
};

HighlightedCode.displayName = "HighlightedCode";
