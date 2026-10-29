// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Whether a PR changes no code, so Codex has nothing to review in it. Decided
// by what the changed files are and, for the version files, by what changed in
// them: never by the PR's title, labels or author. Every changed file has to be
// in one of three classes:
//
//   1. release: the version files, where only version lines changed
//      (package.json, Cargo.toml: the package's own version; package-lock.json,
//      Cargo.lock: the versions of our own packages and crates, not a
//      dependency's) and VERSION_HISTORY.md at the root;
//   2. docs: Markdown (.md, .mdx) in a docs, specs or reports directory, a
//      changeset, or a root README/CHANGELOG/LICENSE/NOTICE;
//   3. assets: images, recordings and fonts.
//
// Anything else, even one line, is code. Files an agent reads as instructions
// (CLAUDE.md, AGENTS.md, skills, prompts) and everything that runs in CI
// (.github/, scripts/, tools/) are never docs, and a dependency change in a
// manifest or a lockfile is not a version line.
//
// muxreview keeps a mirror of this rule: it decides
// not to ask Codex, and this gate checks the same rule again before it passes a
// head on muxreview's word. Keep the two in step.

const NEVER_PREFIXES = [".github/", "scripts/", "tools/", "prompts/", ".claude/", ".codex/", ".agents/", ".cursor/"];
// Markdown that an agent acts on.
const AGENT_FILES = /^(CLAUDE|AGENTS|GEMINI|SKILL)\.(md|mdx)$/i;
const DOC_DIRS = new Set(["docs", "doc", "documentation", "specs", "reports", "docs-site", "docsite", ".changesets", ".changeset"]);
const ROOT_DOC_NAMES = /^((README|CHANGELOG)\.(md|mdx)|(LICENSE|NOTICE)(\.(md|txt))?)$/i;
// SVG is left out on purpose: it can carry script.
const ASSET_EXTENSIONS = new Set([
    "png", "jpg", "jpeg", "gif", "webp", "avif", "ico", "bmp", "icns",
    "mp4", "webm", "mov", "wav", "mp3", "ogg",
    "woff", "woff2", "ttf", "otf", "eot",
]);
const VERSION_FILES = new Set(["package.json", "package-lock.json", "Cargo.toml", "Cargo.lock"]);
const SEMVER = String.raw`\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.+-]+)?`;
const SEMVER_RE = new RegExp(`^${SEMVER}$`);
const CARGO_VERSION_LINE = new RegExp(String.raw`^version\s*=\s*"${SEMVER}"\s*(#.*)?$`);
// The files API lists at most 3000; a list that long may be cut short.
export const PR_FILE_CAP = 3000;

const normalize = (rawPath) => {
    let p = String(rawPath ?? "").trim().replace(/\\/g, "/");
    if (p.startsWith("./")) p = p.slice(2);
    return p;
};
const baseName = (p) => p.split("/").at(-1);
const extension = (p) => (baseName(p).includes(".") ? baseName(p).split(".").at(-1).toLowerCase() : "");

/** Classes 2 and 3, and the history file of class 1: decided by the path alone. */
export function noCodePath(rawPath) {
    const p = normalize(rawPath);
    if (p === "" || NEVER_PREFIXES.some((x) => p.startsWith(x))) return false;
    const name = baseName(p);
    if (!p.includes("/") && name === "VERSION_HISTORY.md") return true;
    if (ASSET_EXTENSIONS.has(extension(p))) return true;
    if (!["md", "mdx"].includes(extension(p)) && !ROOT_DOC_NAMES.test(name)) return false;
    if (AGENT_FILES.test(name)) return false;
    if (!p.includes("/")) return ROOT_DOC_NAMES.test(name);
    return p.split("/").slice(0, -1).some((segment) => DOC_DIRS.has(segment.toLowerCase()));
}

const isObject = (x) => x !== null && typeof x === "object" && !Array.isArray(x);

function deepEqual(a, b) {
    if (a === b) return true;
    if (Array.isArray(a) || Array.isArray(b)) {
        return Array.isArray(a) && Array.isArray(b) && a.length === b.length && a.every((x, i) => deepEqual(x, b[i]));
    }
    if (isObject(a) && isObject(b)) {
        const keys = Object.keys(a);
        return keys.length === Object.keys(b).length && keys.every((k) => k in b && deepEqual(a[k], b[k]));
    }
    return false;
}

function parseJson(text) {
    try {
        const v = JSON.parse(text);
        return isObject(v) ? v : null;
    } catch {
        return null;
    }
}

/** package.json: the same file with a different top-level `version`. */
export function packageJsonVersionOnly(base, head) {
    const a = parseJson(base);
    const b = parseJson(head);
    if (!a || !b || typeof a.version !== "string" || !SEMVER_RE.test(b.version ?? "") || a.version === b.version) return false;
    const { version: _a, ...restA } = a;
    const { version: _b, ...restB } = b;
    return deepEqual(restA, restB);
}

/**
 * A local package is the root ("") or an entry outside node_modules (a workspace member), with no
 * registry `resolved` URL, no bundle flag and no link. Anything under node_modules is a dependency,
 * whatever keys it has or lacks.
 */
const isLocalPackage = (key, entry) =>
    isObject(entry) && entry.resolved === undefined && !entry.inBundle && !entry.link && (key === "" || !key.split("/").includes("node_modules"));

function withoutLocalVersions(lock) {
    const packages = {};
    let changedKeys = 0;
    for (const [key, entry] of Object.entries(lock.packages ?? {})) {
        if (isLocalPackage(key, entry) && "version" in entry) {
            const { version: _v, ...rest } = entry;
            packages[key] = rest;
            changedKeys += 1;
        } else {
            packages[key] = entry;
        }
    }
    const { version: _top, packages: _packages, ...rest } = lock;
    return { rest: { ...rest, packages }, changedKeys };
}

/** package-lock.json: only the versions of the root and of local (workspace) packages differ. */
export function packageLockVersionOnly(base, head) {
    const a = parseJson(base);
    const b = parseJson(head);
    if (!a || !b || a.lockfileVersion === undefined || a.lockfileVersion !== b.lockfileVersion) return false;
    const versions = [
        a.version,
        b.version,
        ...Object.entries(b.packages ?? {}).filter(([k, e]) => isLocalPackage(k, e)).map(([, e]) => e.version),
    ];
    if (versions.some((v) => v !== undefined && !SEMVER_RE.test(String(v)))) return false;
    return deepEqual(withoutLocalVersions(a).rest, withoutLocalVersions(b).rest);
}

const lines = (text) => String(text).replace(/\r\n/g, "\n").split("\n");

/** Cargo.toml: only `version = "x.y.z"` lines of [package] or [workspace.package] differ. */
export function cargoTomlVersionOnly(base, head) {
    const a = lines(base);
    const b = lines(head);
    if (a.length !== b.length) return false;
    let table = "";
    let changed = 0;
    for (let i = 0; i < b.length; i++) {
        // Any line that opens with a bracket may start a table: one this can't read (a trailing
        // comment, an odd form) leaves the table unknown, and nothing is accepted under an unknown table.
        if (/^\s*\[/.test(b[i])) {
            const header = /^\s*\[\[?([^\]]+)\]\]?\s*(#.*)?$/.exec(b[i]);
            table = header ? header[1].trim() : "?";
        }
        if (a[i] === b[i]) continue;
        if (table !== "package" && table !== "workspace.package") return false;
        if (!CARGO_VERSION_LINE.test(a[i]) || !CARGO_VERSION_LINE.test(b[i])) return false;
        changed += 1;
    }
    return changed > 0;
}

/** Cargo.lock: only version lines of packages with no `source` (ours, not a registry's) differ. */
export function cargoLockVersionOnly(base, head) {
    const a = lines(base);
    const b = lines(head);
    if (a.length !== b.length) return false;
    // For each line of head: the [[package]] block it is in, and whether that block has a source.
    const block = new Array(b.length).fill(-1);
    const hasSource = [];
    let current = -1;
    for (let i = 0; i < b.length; i++) {
        if (/^\[\[package\]\]\s*$/.test(b[i])) {
            current = hasSource.length;
            hasSource.push(false);
        } else if (/^\s*\[/.test(b[i])) {
            current = -1;
        } else if (current >= 0 && /^source\s*=/.test(b[i])) {
            hasSource[current] = true;
        }
        block[i] = current;
    }
    let changed = 0;
    for (let i = 0; i < b.length; i++) {
        if (a[i] === b[i]) continue;
        if (block[i] < 0 || hasSource[block[i]]) return false;
        if (!CARGO_VERSION_LINE.test(a[i]) || !CARGO_VERSION_LINE.test(b[i])) return false;
        changed += 1;
    }
    return changed > 0;
}

const VERSION_CHECKS = {
    "package.json": packageJsonVersionOnly,
    "package-lock.json": packageLockVersionOnly,
    "Cargo.toml": cargoTomlVersionOnly,
    "Cargo.lock": cargoLockVersionOnly,
};

/**
 * Whether the PR changes no code. `files` is the pulls files API list (each
 * with `filename`, `status` and, for a rename, `previous_filename`).
 * `readText(path, side)` reads a file as it was before the PR (`"base"`, at the
 * merge base) or after it (`"head"`) and answers null when it can't; it is
 * asked only for the version files. Returns { ok, why }: `why` names the first
 * file that is code, for the log.
 */
export async function isNoCodePr({ files, readText }) {
    if (!Array.isArray(files) || files.length === 0) return { ok: false, why: "no file list" };
    if (files.length >= PR_FILE_CAP) return { ok: false, why: "the file list may be cut short" };
    for (const f of files) {
        const path = normalize(f?.filename);
        // A file moved out of a code path is a change to code, whatever it became.
        if (f?.previous_filename && !noCodePath(f.previous_filename)) return { ok: false, why: `${f.previous_filename} is code` };
        if (noCodePath(path)) continue;
        const check = VERSION_CHECKS[baseName(path)];
        const versionFile = VERSION_FILES.has(baseName(path)) && !NEVER_PREFIXES.some((x) => path.startsWith(x));
        if (!versionFile || !check || f.status !== "modified" || f.previous_filename) return { ok: false, why: `${path} is code` };
        const [base, head] = await Promise.all([readText(path, "base"), readText(path, "head")]);
        if (base == null || head == null) return { ok: false, why: `${path} could not be read` };
        if (!check(base, head)) return { ok: false, why: `${path} changes more than version lines` };
    }
    return { ok: true, why: "every changed file is a version line, docs or an asset" };
}
