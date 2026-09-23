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
 * Usage:
 *   ci-classify-changes.mjs            # newline-separated paths on stdin
 *   ci-classify-changes.mjs a.md b.rs  # or as arguments
 *
 * Prints `key=value` lines suitable for $GITHUB_OUTPUT:
 *   rust=true|false        run the Rust compile/test job
 *   frontend=true|false    run vitest / tsc
 *   docs_only=true|false   every changed file was documentation
 *   docs_index=true|false  run the specs-index generator on all three OSes
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
    /^\.github\/workflows\/ci-pr\.yml$/,
    /^package(-lock)?\.json$/,
    /^(vite|vitest)\.config\./,
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
    const files = fromArgs.length > 0 ? fromArgs : (await readStdin()).split("\n");
    const result = classifyChanges(files);
    console.log(`rust=${result.rust}`);
    console.log(`frontend=${result.frontend}`);
    console.log(`docs_only=${result.docs_only}`);
    console.log(`docs_index=${result.docs_index}`);
    console.error(`ci-classify-changes: ${result.reason}`);
}
