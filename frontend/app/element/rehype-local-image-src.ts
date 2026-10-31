// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { Element, Root } from "hast";
import { visit } from "unist-util-visit";

/** `C:/…`, `C:\…`, or `C:%5C…` (the markdown pipeline percent-encodes `\`). */
const DRIVE_PATH = /^[A-Za-z]:(?:[\\/]|%5C)/i;

/**
 * Rewrites an image `src` or a link `href` that is a Windows drive path to a
 * `file:///` URL. Without this the sanitizer reads `C:` as a URL scheme it
 * doesn't allow and drops the attribute, so `![x](C:/shot.png)` could never
 * load and `[x](C:/notes.md)` rendered as a dead link. Runs before sanitize;
 * `MarkdownImg` and the link click (link-target.ts) turn the URL back into a
 * path.
 */
export function rehypeLocalImageSrc() {
    return (tree: Root) => {
        visit(tree, "element", (node: Element) => {
            const attr = node.tagName === "img" ? "src" : node.tagName === "a" ? "href" : null;
            if (!attr) return;
            const value = node.properties?.[attr];
            if (typeof value !== "string" || !DRIVE_PATH.test(value)) return;
            node.properties[attr] = "file:///" + value.replace(/%5C/gi, "/").replace(/\\/g, "/");
        });
    };
}
