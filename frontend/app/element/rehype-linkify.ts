// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { Root, Element, Text, ElementContent } from "hast";
import type { Plugin } from "unified";
import { visitParents, SKIP } from "unist-util-visit-parents";
import { linkify, normalizeHref, isLikelyFilename } from "./linkify-config";

// Don't linkify text inside these elements — already an anchor, or code/pre
const SKIP_ANCESTORS = new Set(["a", "code", "pre", "script", "style"]);

export const rehypeLinkify: Plugin<[], Root> = () => (tree) => {
    visitParents(tree, "text", (node: Text, ancestors) => {
        // Walk ancestor chain — skip if we're inside a, code, pre, etc.
        for (const ancestor of ancestors) {
            if (ancestor.type === "element" && SKIP_ANCESTORS.has((ancestor as Element).tagName)) {
                return SKIP;
            }
        }

        const matches = linkify.match(node.value);
        if (!matches) return;

        const newNodes: ElementContent[] = [];
        let lastIndex = 0;

        for (const match of matches) {
            // Drop fuzzy matches that are actually source-code filenames
            // (e.g. README.md, main.rs, setup.py treated as ccTLD URLs)
            if (isLikelyFilename(match.schema, match.url)) {
                if (match.index > lastIndex) {
                    newNodes.push({ type: "text", value: node.value.slice(lastIndex, match.index) });
                }
                newNodes.push({ type: "text", value: match.raw });
                lastIndex = match.lastIndex;
                continue;
            }

            if (match.index > lastIndex) {
                newNodes.push({ type: "text", value: node.value.slice(lastIndex, match.index) });
            }
            newNodes.push({
                type: "element",
                tagName: "a",
                properties: { href: normalizeHref(match.url) },
                children: [{ type: "text", value: match.text }],
            });
            lastIndex = match.lastIndex;
        }

        if (lastIndex < node.value.length) {
            newNodes.push({ type: "text", value: node.value.slice(lastIndex) });
        }

        // If nothing was actually linkified, bail out without mutating the tree
        if (newNodes.length === 1 && newNodes[0].type === "text") return;

        const parent = ancestors[ancestors.length - 1] as Element;
        const index = parent.children.indexOf(node as ElementContent);
        parent.children.splice(index, 1, ...newNodes);
        // Jump past our inserted nodes so they aren't re-visited
        return [SKIP, index + newNodes.length];
    });
};
