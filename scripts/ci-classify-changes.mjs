#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Decide which CI jobs a PR's changed files actually require.
 *
 * Implements docs/specs/SPEC_CI_PATH_CONDITIONAL_CHECKS_2026_09_21.md.
 *
 * This is safety-critical in a way that is easy to miss. A job skipped by an
 * `if:` conditional reports "skipped", and GitHub counts skipped as PASSING
 * for a required status check. So a wrong answer here does not fail loudly —
 * it puts a green check on code nobody compiled. Every rule below biases
 * toward running:
 *
 *   R1  "ALL files are docs", never "ANY file is a doc". One source file in an
 *       otherwise-documentation PR must run everything.
 *   R2  Anything unrecognised, malformed or empty means RUN. The worst
 *       outcome allowed is a wasted six minutes.
 *   R3  CI configuration is never documentation — a PR editing the test setup
 *       must not be able to skip the tests it edits.
 *
 *   R7  A PR that only moves the release version (a release PR) needs none
 *       of the build jobs: every changed file is documentation or a version
 *       manifest, and every changed manifest line is that one version moving
 *       from A to B. Proven from the diff itself, never from a path or a
 *       title, so a "release" PR that also bumps a dependency runs everything.
 *
 * Usage:
 *   ci-classify-changes.mjs            # newline-separated paths on stdin
 *   ci-classify-changes.mjs a.md b.rs  # or as arguments
 *   ... | ci-classify-changes.mjs      # or a JSON array of {filename, patch}
 *                                      # (the PR files API), which R7 needs
 *
 * Prints `key=value` lines suitable for $GITHUB_OUTPUT:
 *   rust=true|false          run the Rust compile/test job
 *   frontend=true|false      run vitest / tsc
 *   docs_only=true|false     every changed file was documentation
 *   docs_index=true|false    run the specs-index generator on all three OSes
 *   version_only=true|false  the PR only moves the release version (R7)
 */

/**
 * Paths that can change what scripts/gen-docs-index.mjs produces on some OS, or
 * how its cross-platform job runs. Its output is a pure function of file bytes,
 * so it can only start to differ between platforms when the generator, its
 * tests, their runtime (Node/vitest config and dependencies) or the job itself
 * changes — not when a spec does. Specs are still asserted on every PR by the
 * Linux `docs` job. docs/specs/SPEC_DOCS_INDEX_GENERATOR_NODE_PORT_2026_09_23.md §7.
 */
const DOCS_INDEX_PATTERNS = [
    /^scripts\/gen-docs-index\./,
    /^scripts\/test-fixtures\/gen-docs-index\//,
    /^scripts\/ci-classify-changes\./,
    // Its CLI-free tests run in this job's step (ci-pr.yml).
    /^scripts\/cli-probe\//,
    /^\.github\/workflows\/ci-pr\.yml$/,
    /^package(-lock)?\.json$/,
    /^(vite|vitest)\.config\./,
    // Attributes decide the bytes a checkout gives the generator (eol, text),
    // so changing them can make a committed INDEX.md unreproducible on some OS
    // with no spec or generator touched (Codex P2, #3590).
    /^\.gitattributes$/,
];

/**
 * Paths that cannot affect a build. Deliberately short: every entry here is a
 * promise that no compiler, bundler or test runner reads this file.
 *
 * `docs/**` is safe to list because the `docs` job always runs regardless of
 * this classification — documentation still gets its own gates.
 */
const DOCS_ONLY_PATTERNS = [
    /^docs\/.*$/,
    /^\.changesets\/.*$/,
    /\.mdx?$/i,
    /^LICENSE$/,
    /^NOTICE$/,
    /^CODE_OF_CONDUCT(\.[a-z]+)?$/i,
    /^CONTRIBUTING(\.[a-z]+)?$/i,
];

/**
 * R3. Checked BEFORE the docs patterns, so a `.md` file under `.github/` (an
 * issue template, a workflow's README) can never be classified as harmless.
 * These directories configure what CI does; a change to them must exercise it.
 */
/**
 * `.changesets/` is deliberately absent — it belongs in DOCS_ONLY_PATTERNS
 * above. A `.changeset/` (singular) entry sat here and was dead code: no such
 * directory exists in this repo (reagentx P2, PR #3491).
 *
 * **Do not "correct the typo" by moving `.changesets/` into this list.** Every
 * PR carries a changeset entry, so classifying them as never-docs would force
 * a full build on every PR and this mechanism would never fire once. Verified
 * 2026-09-21: `.changesets/` is read only by `release.sh`, `package*.sh`,
 * `bump-wrapper.sh`, `dev-local.sh`, `gen-seed.js` and `nightly-release.yml`
 * — none of which run in the PR lane.
 */
const NEVER_DOCS_PREFIXES = [".github/", "scripts/", "tools/"];
const NEVER_DOCS_EXACT = ["Taskfile.yml", "Taskfile.yaml", ".gitattributes", ".gitignore"];

/** Normalize a path the way git reports it, tolerating quoting and `\` separators. */
function normalize(rawPath) {
    let p = String(rawPath ?? "").trim();
    if (p.startsWith('"') && p.endsWith('"') && p.length >= 2) {
        p = p.slice(1, -1);
    }
    return p.replace(/\\/g, "/").replace(/^\.\//, "");
}

/**
 * The files a release bumps (scripts/release.sh via bump-wrapper.sh), each with
 * the shape of its version line. Only these, at the repo root: a crate's own
 * Cargo.toml inherits the workspace version and never changes in a release.
 */
const VERSION_MANIFESTS = {
    "Cargo.toml": /^\s*version\s*=\s*"([^"]+)"\s*$/,
    "Cargo.lock": /^\s*version\s*=\s*"([^"]+)"\s*$/,
    "package.json": /^\s*"version"\s*:\s*"([^"]+)"\s*,?\s*$/,
    "package-lock.json": /^\s*"version"\s*:\s*"([^"]+)"\s*,?\s*$/,
};

/**
 * R7: whether the changed files only move the release version, from their
 * unified-diff patches (the PR files API's `patch`). Requires, all at once:
 *  - every file is documentation (isDocsOnlyPath) or a VERSION_MANIFESTS file;
 *  - package.json is among them (it carries the release version), and every
 *    manifest has a patch (the API omits it for a huge or binary diff);
 *  - every added or removed line of every manifest is a version line;
 *  - every removed version is the same value A and every added one the same
 *    value B, A != B, as many added as removed. A dependency bump has its
 *    own versions (and a lockfile checksum line), so it can't pass.
 * Returns { ok: true, from, to } or { ok: false, reason }.
 */
export function versionOnlyChange(entries) {
    const list = Array.isArray(entries) ? entries : [];
    const manifests = [];
    for (const e of list) {
        const filename = normalize(typeof e === "string" ? e : e?.filename);
        if (filename === "") return { ok: false, reason: "an entry has no file name" };
        if (VERSION_MANIFESTS[filename]) {
            manifests.push({ filename, patch: typeof e === "object" ? e.patch : undefined });
        } else if (!isDocsOnlyPath(filename)) {
            return { ok: false, reason: `${filename} is neither documentation nor a version manifest` };
        }
    }
    if (!manifests.some((m) => m.filename === "package.json")) return { ok: false, reason: "package.json's version doesn't change" };
    const removed = new Set();
    const added = new Set();
    let removedCount = 0;
    let addedCount = 0;
    for (const { filename, patch } of manifests) {
        if (typeof patch !== "string" || patch === "") return { ok: false, reason: `no diff available for ${filename}` };
        for (const line of patch.split(/\r?\n/)) {
            if (line.startsWith("@@") || line === "" || line.startsWith(" ") || line.startsWith("\\")) continue;
            const sign = line[0];
            if (sign !== "+" && sign !== "-") return { ok: false, reason: `unexpected diff line in ${filename}` };
            const m = VERSION_MANIFESTS[filename].exec(line.slice(1));
            if (!m) return { ok: false, reason: `${filename} changes more than its version` };
            if (sign === "-") {
                removed.add(m[1]);
                removedCount++;
            } else {
                added.add(m[1]);
                addedCount++;
            }
        }
    }
    if (removed.size !== 1 || added.size !== 1 || removedCount !== addedCount || removedCount === 0) {
        return { ok: false, reason: "the version lines don't all move from one value to one other" };
    }
    const [from] = removed;
    const [to] = added;
    if (from === to) return { ok: false, reason: "the version doesn't change" };
    // A value Cargo and npm would reject still agrees across the files, and the
    // release consistency check only checks agreement: build it (Codex, #4494).
    for (const v of [from, to]) {
        if (!SEMVER.test(v)) return { ok: false, reason: `"${v}" isn't a valid version` };
    }
    return { ok: true, from, to };
}

/** A semantic version as Cargo and npm accept it: X.Y.Z, optional -pre and +build. */
const SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$/;

/** True when this single path provably cannot affect any build output. */
export function isDocsOnlyPath(rawPath) {
    const p = normalize(rawPath);
    if (p === "") return false; // R2
    if (NEVER_DOCS_EXACT.includes(p)) return false; // R3
    if (NEVER_DOCS_PREFIXES.some((prefix) => p.startsWith(prefix))) return false; // R3
    return DOCS_ONLY_PATTERNS.some((re) => re.test(p));
}

/**
 * Classify a whole changed-file list.
 *
 * An empty list is NOT treated as "nothing changed, skip everything" — an
 * empty list is far more likely to mean the API call failed or returned
 * something unexpected, so it runs everything (R2).
 */
export function classifyChanges(files) {
    const paths = (Array.isArray(files) ? files : [])
        .map(normalize)
        .filter((p) => p !== "");

    if (paths.length === 0) {
        return {
            rust: true,
            frontend: true,
            docs_only: false,
            docs_index: true,
            reason: "no files resolved — running everything (R2)",
        };
    }

    // R2 for the index job too: an entry that is not a string means the input
    // was not what this script expects, so run it rather than trust the rest.
    const malformed = files.some((f) => typeof f !== "string");
    const docs_index = malformed || paths.some((p) => DOCS_INDEX_PATTERNS.some((re) => re.test(p)));

    const nonDocs = paths.filter((p) => !isDocsOnlyPath(p));
    if (nonDocs.length === 0) {
        return {
            rust: false,
            frontend: false,
            docs_only: true,
            docs_index,
            reason: `all ${paths.length} changed file(s) are documentation`,
        };
    }

    return {
        rust: true,
        frontend: true,
        docs_only: false,
        docs_index,
        // Naming a concrete file makes a wrong decision debuggable from the log
        // alone, without re-deriving the whole list.
        reason: `${nonDocs.length} non-doc file(s), e.g. ${nonDocs[0]}`,
    };
}

/**
 * Classify the PR files API's entries ({filename, patch}). A version-only PR
 * (R7) skips the build jobs; anything else is classified by its paths exactly
 * as classifyChanges does, and an entry without a string filename is malformed
 * (R2: run everything).
 */
export function classifyPullFiles(entries) {
    const list = Array.isArray(entries) ? entries : [];
    const names = list.map((e) => (e && typeof e === "object" && typeof e.filename === "string" ? e.filename : null));
    if (list.length === 0 || names.some((n) => n === null)) {
        return { ...classifyChanges([]), version_only: false };
    }
    const version = versionOnlyChange(list);
    if (version.ok) {
        return {
            rust: false,
            frontend: false,
            docs_only: false,
            docs_index: false,
            version_only: true,
            reason: `version-only change, ${version.from} -> ${version.to} (R7)`,
        };
    }
    const byPath = classifyChanges(names);
    return { ...byPath, version_only: false, reason: `${byPath.reason}; not version-only: ${version.reason}` };
}

async function readStdin() {
    if (process.stdin.isTTY) return "";
    let data = "";
    for await (const chunk of process.stdin) data += chunk;
    return data;
}

// Entry point only when executed directly, so the test can import the pure
// functions above without this running.
const isMain = process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/\\/g, "/").split("/").pop());
if (isMain) {
    const fromArgs = process.argv.slice(2);
    const input = fromArgs.length > 0 ? null : await readStdin();
    let result;
    if (input !== null && input.trimStart().startsWith("[")) {
        // The PR files API's entries, with patches (R7). Unparseable JSON is
        // malformed input: run everything (R2).
        let entries = null;
        try {
            entries = JSON.parse(input);
        } catch {
            entries = null;
        }
        result = classifyPullFiles(entries);
    } else {
        result = { ...classifyChanges(fromArgs.length > 0 ? fromArgs : input.split("\n")), version_only: false };
    }
    console.log(`rust=${result.rust}`);
    console.log(`frontend=${result.frontend}`);
    console.log(`docs_only=${result.docs_only}`);
    console.log(`docs_index=${result.docs_index}`);
    console.log(`version_only=${result.version_only}`);
    console.error(`ci-classify-changes: ${result.reason}`);
}
