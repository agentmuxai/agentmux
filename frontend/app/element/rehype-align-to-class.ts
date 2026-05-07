// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { visit } from "unist-util-visit";
import type { Root, Element } from "hast";

const ALIGN_CLASS: Record<string, string> = {
    left: "text-left",
    center: "text-center",
    right: "text-right",
};

// Sanitize allowlist for the classes this plugin emits. Both markdown.tsx
// (which configures rehype-sanitize directly) and streamdown.tsx (which
// replaces streamdown's default sanitize plugin) must merge this into their
// schema; otherwise sanitize strips the className before custom th/td
// renderers can read it (codex P2 PR #718).
export const ALIGN_CLASS_REGEX = /^text-(left|center|right)$/;

// Rehype plugin: converts remark-gfm's deprecated `align` attribute on <th>/<td>
// into a Tailwind class before rehype-sanitize strips it. Must run before sanitize.
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
