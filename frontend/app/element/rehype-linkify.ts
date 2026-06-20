// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { Root, Element, Text, ElementContent } from "hast";
import type { Plugin } from "unified";
import { visitParents, SKIP } from "unist-util-visit-parents";
import LinkifyIt from "linkify-it";

const linkify = new LinkifyIt();
// fuzzyLink matches bare domains (github.com, localhost:3000) without http://
// fuzzyEmail off to avoid false positives on name@host patterns in shell output
linkify.set({ fuzzyLink: true, fuzzyEmail: false });

// Don't linkify text inside these elements — already an anchor, or code/pre
const SKIP_ANCESTORS = new Set(["a", "code", "pre", "script", "style"]);

// Local addresses use http://, not https://
const LOCAL_HOST_RE = /^(localhost|127\.\d+\.\d+\.\d+|0\.0\.0\.0|\[::1\])(:\d+)?$/i;

function normalizeHref(url: string): string {
    if (url.startsWith("//")) return "https:" + url;
    if (url.includes("://")) return url;
    const scheme = LOCAL_HOST_RE.test(url.split("/")[0]) ? "http" : "https";
    return `${scheme}://${url}`;
}

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

        const parent = ancestors[ancestors.length - 1] as Element;
        const index = parent.children.indexOf(node as ElementContent);
        parent.children.splice(index, 1, ...newNodes);
        // Jump past our inserted nodes so they aren't re-visited
        return [SKIP, index + newNodes.length];
    });
};
