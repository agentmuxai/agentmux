// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Marks each fenced block's `<code>` (a `<code>` directly inside a `<pre>`,
 * with a `language-x` class) with `data-highlight="x"`. The markdown `Code`
 * component colours a block so marked with shiki once it loads
 * (markdown-codeblock.tsx). Inline code and blocks without a language stay
 * plain, as with rehype-highlight, which never guessed a language.
 *
 * Runs after the sanitizer, which would drop the attribute; it only adds one
 * attribute derived from a class the sanitizer already let through.
 * SPEC_MARKDOWN_CODE_ON_SHIKI_AND_REDUNDANT_LIBRARIES_2026_10_21.md §4.1.
 */

interface HastNode {
    type: string;
    tagName?: string;
    properties?: Record<string, unknown>;
    children?: HastNode[];
}

const LANGUAGE_CLASS = /^language-(.+)$/;

/** The `x` of a `language-x` class, if `code` has one. */
export function codeLanguage(code: HastNode): string | undefined {
    const classes = code.properties?.className;
    const list = Array.isArray(classes) ? classes : typeof classes === "string" ? classes.split(/\s+/) : [];
    for (const c of list) {
        const m = LANGUAGE_CLASS.exec(String(c));
        if (m) return m[1];
    }
    return undefined;
}

function mark(node: HastNode): void {
    for (const child of node.children ?? []) {
        if (node.type === "element" && node.tagName === "pre" && child.type === "element" && child.tagName === "code") {
            const lang = codeLanguage(child);
            if (lang) child.properties = { ...child.properties, dataHighlight: lang };
        }
        mark(child);
    }
}

export function rehypeMarkCodeLang() {
    return (tree: HastNode) => mark(tree);
}
