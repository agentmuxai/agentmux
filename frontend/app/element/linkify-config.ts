// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Shared linkify-it instance and helpers used by both rehype-linkify.ts
// (markdown pipeline) and linkified-text.tsx (plain-text nodes).

import LinkifyIt from "linkify-it";

export const linkify = new LinkifyIt();
// fuzzyLink: bare domains (github.com, localhost:3000) detected without http://
// fuzzyEmail off: avoids false positives on name@host patterns in shell output
linkify.set({ fuzzyLink: true, fuzzyEmail: false });

// Local addresses open with http://, not https://
const LOCAL_HOST_RE = /^(localhost|127\.\d+\.\d+\.\d+|0\.0\.0\.0|\[::1\])(:\d+)?$/i;

export function normalizeHref(url: string): string {
    if (url.startsWith("//")) return "https:" + url;
    if (url.includes("://")) return url;
    const host = url.split("/")[0].split(":")[0];
    const scheme = LOCAL_HOST_RE.test(host) ? "http" : "https";
    return `${scheme}://${url}`;
}

// ccTLDs that are also ubiquitous source-code file extensions. fuzzyLink
// would turn `README.md`, `main.rs`, `setup.py`, `build.sh` into bogus
// external links (https://README.md etc.). We reject any fuzzy match whose
// entire "hostname" is a single bare word + one of these extensions.
const FILE_EXT_TLDS = new Set(["md", "rs", "py", "sh", "pl", "ml"]);

export function isLikelyFilename(schema: string, url: string): boolean {
    // Matches with an explicit protocol are never filenames
    if (schema) return false;
    // Strip path/port to get the bare host (e.g. "README.md" from "README.md/foo")
    const host = url.split("/")[0].split(":")[0];
    const dotIdx = host.lastIndexOf(".");
    if (dotIdx === -1) return false;
    const ext = host.slice(dotIdx + 1).toLowerCase();
    // Only filter single-label hosts (one dot total): "README.md" yes,
    // "example.io" or "api.github.md" both have more dots and are real URLs.
    const dotsInHost = (host.match(/\./g) ?? []).length;
    return dotsInHost === 1 && FILE_EXT_TLDS.has(ext);
}

export const SAFE_SCHEMES = /^(https?|ftp|mailto|ssh|file):\/\//i;

export function isSafeHref(url: string): boolean {
    return SAFE_SCHEMES.test(url) || url.startsWith("//");
}
