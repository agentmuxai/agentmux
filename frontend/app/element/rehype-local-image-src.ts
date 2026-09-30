// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { Element, Root } from "hast";
import { visit } from "unist-util-visit";

/** `C:/…`, `C:\…`, or `C:%5C…` (the markdown pipeline percent-encodes `\`). */
const DRIVE_PATH = /^[A-Za-z]:(?:[\\/]|%5C)/i;

/**
 * Rewrites an image `src` that is a Windows drive path to a `file:///` URL.
 * Without this the sanitizer reads `C:` as a URL scheme it doesn't allow and
 * drops the `src`, so `![x](C:/shot.png)` could never load. Runs before
 * sanitize; `MarkdownImg` turns the URL back into a path.
 */
export function rehypeLocalImageSrc() {
    return (tree: Root) => {
        visit(tree, "element", (node: Element) => {
            if (node.tagName !== "img") return;
            const src = node.properties?.src;
            if (typeof src !== "string" || !DRIVE_PATH.test(src)) return;
            node.properties.src = "file:///" + src.replace(/%5C/gi, "/").replace(/\\/g, "/");
        });
    };
}
