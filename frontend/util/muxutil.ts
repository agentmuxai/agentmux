// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0s

import { getWebServerEndpoint } from "@/util/endpoints";
import { boundNumber, isBlank } from "@/util/util";
import { createSignal } from "solid-js";

// The CSS parser for a background with `url(...)`, loaded with the first such
// background rather than at startup (SPEC_FRONTEND_BUNDLE_SPLITTING_2026_10_21.md §5).
// The signals let a caller's memo compute the style again once it arrives, or
// once a failed load may be tried again (after 2, 4, 8… seconds, at most 60
// apart, for as long as a background waits for it).
let cssTree: typeof import("css-tree") | null = null;
let cssTreeLoading = false;
let cssTreeFailures = 0;
const CSS_TREE_RETRY_MAX_MS = 60_000;
const [cssTreeLoaded, setCssTreeLoaded] = createSignal(false);
const [cssTreeRetry, setCssTreeRetry] = createSignal(0);

function loadCssTree(): void {
    if (cssTreeLoading) return;
    cssTreeLoading = true;
    import("css-tree")
        .then((m) => {
            cssTree = m;
            setCssTreeLoaded(true);
        })
        .catch((e) => {
            cssTreeLoading = false;
            console.error("error loading the background parser", e);
            cssTreeFailures++;
            // Waiting backgrounds read `cssTreeRetry`: their memos run again and load again.
            const delay = Math.min(1000 * 2 ** cssTreeFailures, CSS_TREE_RETRY_MAX_MS);
            setTimeout(() => setCssTreeRetry((n) => n + 1), delay);
        });
}

function encodeFileURL(file: string) {
    const webEndpoint = getWebServerEndpoint();
    const fileUri = formatRemoteUri(file, "local");
    const rtn = webEndpoint + `/agentmux/stream-file?path=${encodeURIComponent(fileUri)}&no404=1`;
    return rtn;
}

function processBackgroundUrls(cssText: string): string {
    if (isBlank(cssText)) {
        return null;
    }
    cssText = cssText.trim();
    if (cssText.endsWith(";")) {
        cssText = cssText.slice(0, -1);
    }
    const attrRe = /^background(-image)?\s*:\s*/i;
    cssText = cssText.replace(attrRe, "");
    // Only a value that can load something (`url()`, `src()`, `image()`,
    // `image-set()`) needs the parser to vet and rewrite it; anything else (a
    // colour, a gradient) is used as written. A backslash always goes to the
    // parser: CSS escapes can spell a function name (`\75rl(` is `url(`).
    if (!cssText.includes("\\") && !/(url|src|image(-set)?)\s*\(/i.test(cssText)) {
        return cssText;
    }
    if (!cssTreeLoaded() || cssTree == null) {
        // No background until the parser arrives; the caller's memo runs again then.
        cssTreeRetry();
        loadCssTree();
        return null;
    }
    const { generate: generateCSS, parse: parseCSS, walk: walkCSS } = cssTree;
    const ast = parseCSS("background: " + cssText, {
        context: "declaration",
    });
    let hasUnsafeUrl = false;
    // What the parser couldn't read, or a name spelled with escapes, may be a
    // function it didn't recognise as one: a browser decodes `\75rl(a.png)`
    // to `url(a.png)`, which the parser leaves as raw text. Refuse those. A
    // backslash inside a URL itself (a Windows path) is fine.
    walkCSS(ast, (node) => {
        const name = (node as { name?: string }).name;
        if (node.type === "Raw" || (typeof name === "string" && name.includes("\\"))) {
            hasUnsafeUrl = true;
        }
    });
    if (hasUnsafeUrl) {
        console.log("Invalid background, contains CSS the parser could not read or an escaped name");
        return null;
    }
    walkCSS(ast, {
        visit: "Url",
        enter(node) {
            const originalUrl = node.value.trim();
            if (
                originalUrl.startsWith("http:") ||
                originalUrl.startsWith("https:") ||
                originalUrl.startsWith("data:")
            ) {
                return;
            }
            // allow file:/// urls (if they are absolute)
            if (originalUrl.startsWith("file://")) {
                const path = originalUrl.slice(7);
                if (!path.startsWith("/")) {
                    console.log(`Invalid background, contains a non-absolute file URL: ${originalUrl}`);
                    hasUnsafeUrl = true;
                    return;
                }
                const newUrl = encodeFileURL(path);
                node.value = newUrl;
                return;
            }
            // allow absolute paths
            if (originalUrl.startsWith("/") || originalUrl.startsWith("~/") || /^[a-zA-Z]:(\/|\\)/.test(originalUrl)) {
                const newUrl = encodeFileURL(originalUrl);
                node.value = newUrl;
                return;
            }
            hasUnsafeUrl = true;
            console.log(`Invalid background, contains an unsafe URL scheme: ${originalUrl}`);
        },
    });
    if (hasUnsafeUrl) {
        return null;
    }
    const rtnStyle = generateCSS(ast);
    if (rtnStyle == null) {
        return null;
    }
    return rtnStyle.replace(/^background:\s*/, "");
}

export function computeBgStyleFromMeta(meta: MetaType, defaultOpacity: number = null): Record<string, string | number> {
    const bgAttr = meta?.["bg"];
    if (isBlank(bgAttr)) {
        return null;
    }
    try {
        const processedBg = processBackgroundUrls(bgAttr);
        const rtn: Record<string, string | number> = {};
        rtn.background = processedBg;
        rtn.opacity = boundNumber(meta["bg:opacity"], 0, 1) ?? defaultOpacity;
        if (!isBlank(meta?.["bg:blendmode"])) {
            rtn.backgroundBlendMode = meta["bg:blendmode"];
        }
        return rtn;
    } catch (e) {
        console.error("error processing background", e);
        return null;
    }
}

function formatRemoteUri(path: string, connection: string): string {
    connection = connection ?? "local";
    // TODO: We need a better way to handle s3 paths
    let retVal: string;
    if (connection.startsWith("aws:")) {
        retVal = `${connection}:s3://${path ?? ""}`;
    } else {
        retVal = `wsh://${connection}/${path}`;
    }
    return retVal;
}
