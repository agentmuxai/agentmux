// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Shared linkify-it instance and helpers used by both rehype-linkify.ts
// (markdown pipeline) and linkified-text.tsx (plain-text nodes).

import LinkifyIt from "linkify-it";

export const linkify = new LinkifyIt();
// fuzzyLink: bare domains (github.com, localhost:3000) detected without http://
// fuzzyEmail off: avoids false positives on name@host patterns in shell output
linkify.set({ fuzzyLink: true, fuzzyEmail: false });

// Local addresses stay on http://, everything else is upgraded to https://
const LOCAL_HOST_RE = /^(localhost|127\.\d+\.\d+\.\d+|0\.0\.0\.0|\[::1\])(:\d+)?$/i;

/**
 * Normalize a linkify-it match to a final href.
 *
 * linkify-it always populates match.url with an absolute URL (it prepends
 * "http://" to schemeless fuzzy matches), so the raw url is always usable.
 * We only need to act when schema is empty (fuzzy match) and the user typed a
 * bare public domain — in that case linkify-it already prepended "http://" but
 * we want to serve "https://" for security. Local addresses keep "http://".
 *
 * Explicit-schema matches (http://, https://, mailto:, ssh:, …) are returned
 * unchanged — the user or model typed a real URL and we respect it.
 */
export function normalizeHref(schema: string, url: string): string {
    // Explicit schema typed by user/model — honour it as-is
    if (schema) return url;
    // Fuzzy match: linkify-it prepended "http://"; upgrade public hosts to https://
    if (url.startsWith("http://")) {
        const host = url.slice(7).split("/")[0].split(":")[0];
        if (!LOCAL_HOST_RE.test(host)) {
            return "https://" + url.slice(7);
        }
    }
    return url;
}

// ccTLDs that double as common source-code file extensions.
const FILE_EXT_TLDS = new Set(["md", "rs", "py", "sh", "pl", "ml"]);

// Generic filename stem words. Combined with FILE_EXT_TLDS, these let us
// reject `main.rs` / `README.md` / `setup.py` while keeping real single-label
// domains like `docs.rs`, `pkg.sh`, `rustup.rs`.
const FILENAME_STEMS = new Set([
    "main", "build", "setup", "index", "mod", "lib", "test", "spec", "app",
    "utils", "types", "constants", "config", "package", "requirements",
    "cargo", "dockerfile", "vagrantfile", "makefile", "gemfile", "pipfile",
    "procfile", "gruntfile", "gulpfile", "webpack", "rollup", "vite",
    "init", "run", "start", "install", "update", "deploy", "release",
]);

export function isLikelyFilename(schema: string, url: string): boolean {
    // Matches with an explicit protocol are real URLs
    if (schema) return false;
    // linkify-it prepends http:// to fuzzy matches — strip it to get the host
    const schemeless = url.startsWith("http://") ? url.slice(7) : url;
    const host = schemeless.split("/")[0].split(":")[0];
    const dotIdx = host.lastIndexOf(".");
    if (dotIdx === -1) return false;
    const ext = host.slice(dotIdx + 1).toLowerCase();
    // Only FILE_EXT_TLDS trigger this check — .com/.io/.dev etc. are fine
    if (!FILE_EXT_TLDS.has(ext)) return false;
    // Only single-label hosts (one dot): "docs.rs" yes, "api.docs.rs" no
    const dotsInHost = (host.match(/\./g) ?? []).length;
    if (dotsInHost !== 1) return false;
    const label = host.slice(0, dotIdx);
    // All-uppercase label → always a filename (README, CHANGELOG, LICENSE)
    if (label === label.toUpperCase() && label.length > 1) return true;
    // Known generic filename stems (main, build, setup, …)
    return FILENAME_STEMS.has(label.toLowerCase());
}

// Allow-list of schemes safe to pass to openExternal. No :// requirement —
// mailto: and ssh: are valid without it.
export function isSafeHref(url: string): boolean {
    return /^(https?|ftp|mailto|ssh|file):/i.test(url) || url.startsWith("//");
}
