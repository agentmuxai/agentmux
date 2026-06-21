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

// Matches any RFC 3986 scheme prefix (http:, https:, mailto:, ftp:, ssh:, …)
const HAS_SCHEME_RE = /^[a-z][a-z0-9+.-]*:/i;

export function normalizeHref(url: string): string {
    if (url.startsWith("//")) return "https:" + url;
    if (HAS_SCHEME_RE.test(url)) return url;
    const host = url.split("/")[0].split(":")[0];
    const scheme = LOCAL_HOST_RE.test(host) ? "http" : "https";
    return `${scheme}://${url}`;
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
    // Strip path/port to get the bare host (e.g. "main.rs" from "main.rs/foo")
    const host = url.split("/")[0].split(":")[0];
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

export const SAFE_SCHEMES = /^(https?|ftp|mailto|ssh|file):\/\//i;

export function isSafeHref(url: string): boolean {
    return SAFE_SCHEMES.test(url) || url.startsWith("//");
}
