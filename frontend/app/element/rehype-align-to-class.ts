// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { visit } from "unist-util-visit";
import type { Root, Element } from "hast";

const ALIGN_CLASS: Record<string, string> = {
    left: "text-left",
    center: "text-center",
    right: "text-right",
};

// Sanitize allowlist for the classes this plugin emits. Used by markdown.tsx,
// which runs this plugin BEFORE its rehype-sanitize call and so must allow
// the resulting className through. The streamdown path runs this plugin
// AFTER streamdown's default sanitize and does not need a schema extension.
export const ALIGN_CLASS_REGEX = /^text-(left|center|right)$/;

// Rehype plugin: converts remark-gfm's deprecated `align` attribute on <th>/<td>
// into a Tailwind class. Two valid placements:
//   - BEFORE rehype-sanitize, when the sanitize schema explicitly allows the
//     emitted className on th/td (markdown.tsx).
//   - AFTER rehype-sanitize, since `align` is in hast-util-sanitize's default
//     global attribute allowlist and survives the sanitize step (streamdown.tsx).
export function rehypeAlignToClass() {
    return (tree: Root) => {
        visit(tree, "element", (node: Element) => {
            if (node.tagName !== "th" && node.tagName !== "td") return;
            const align = node.properties?.align as string | undefined;
            if (!align) return;
            const cls = ALIGN_CLASS[align];
            if (!cls) return;
            const existing = node.properties.className;
            const existingArr: (string | number)[] = Array.isArray(existing)
                ? (existing as (string | number)[])
                : existing && typeof existing !== "boolean"
                ? [existing as string | number]
                : [];
            node.properties.className = [...existingArr, cls];
            delete node.properties.align;
        });
    };
}
