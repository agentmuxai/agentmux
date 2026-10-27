#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The release gate: decides whether the commit a `v*` tag points to may be
// built, signed and published. Run by release-core.yml (the reusable release
// workflow) from a checkout of main, never from the tagged commit, so a tag
// cannot bring its own gate.
//
//   node scripts/release-gate.mjs --ref-type tag --ref-name v0.60.3 \
//        --sha <40 hex> --repo agentmuxai/agentmux [--github-output <file>]
//
// Every input comes from GitHub's own context for the run (github.ref_type,
// github.ref_name, github.sha, github.repository), which a caller workflow
// cannot set. The checks, all of which must hold:
//   1. the run is for a tag named vMAJOR.MINOR.PATCH;
//   2. the commit is on main (the compare API says identical or behind);
//   3. its subject is `chore: release vX.Y.Z`, optionally preceded by the
//      agent's PR-title prefix (`AgentY@narko: `) and followed by ` (#123)`
//      (a squash merge adds the pull request number);
//   4. it changes nothing but what a release changes: the five version files
//      (all five, modified) and deleted `.changesets/*.md` files;
//   5. the five version locations at that commit all equal the tag's version.
// Why 4: a release commit then carries no code of its own. Its tree is its
// parent's (already on main, so merged through the merge queue with a review
// and the required checks) plus the version bump.
//
// Exits non-zero with every reason on stderr when any check fails: a commit
// that is not a release commit on main is never built, signed or published.

import { execFileSync } from "node:child_process";
import { appendFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

export const VERSION_FILES = ["Cargo.lock", "Cargo.toml", "VERSION_HISTORY.md", "package-lock.json", "package.json"];

const TAG_RE = /^v(\d+\.\d+\.\d+)$/;
const SHA_RE = /^[0-9a-f]{40}$/;

const escapeRegex = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** The version each of the five locations holds, from their file contents. */
export function parseVersions(texts) {
    const pkg = JSON.parse(texts["package.json"]).version;
    const pkgLock = JSON.parse(texts["package-lock.json"]).version;
    // First `version` line of Cargo.toml: `version = "x"`.
    const cargoLine = texts["Cargo.toml"].split("\n").find((l) => l.startsWith("version"));
    const cargo = cargoLine?.replace(/^.*=\s*"/, "").replace(/".*$/, "");
    // First `## ` heading of VERSION_HISTORY.md: `## 0.60.3 ...` -> its second field.
    const vhLine = texts["VERSION_HISTORY.md"].split("\n").find((l) => l.startsWith("## "));
    const vh = vhLine?.split(/\s+/)[1];
    // The workspace member agentmux-cef in Cargo.lock: the first `version =` after its `name =`.
    const lines = texts["Cargo.lock"].split("\n");
    const at = lines.findIndex((l) => l === 'name = "agentmux-cef"');
    const lockLine = at < 0 ? undefined : lines.slice(at + 1).find((l) => l.startsWith("version ="));
    const lock = lockLine?.replace(/[" ]/g, "").replace(/^version=/, "");
    return { "package.json": pkg, "Cargo.toml": cargo, "VERSION_HISTORY.md": vh, "Cargo.lock": lock, "package-lock.json": pkgLock };
}

/**
 * Pure decision. `commit` is what the GitHub API says about the commit:
 * `{ status: <compare main...sha>, subject, files: [{ status, filename }] }`;
 * `versions` is parseVersions() of the five files at the commit.
 * Returns { ok, version, reasons }.
 */
export function evaluateGate({ refType, refName, sha, commit, versions }) {
    const reasons = [];
    const tag = TAG_RE.exec(refName ?? "");
    if (refType !== "tag") reasons.push(`the run is for a ${refType || "unknown"} ref, not a tag`);
    if (!tag) reasons.push(`'${refName}' is not a tag named vMAJOR.MINOR.PATCH`);
    if (!SHA_RE.test(sha ?? "")) reasons.push(`'${sha}' is not a full commit sha`);
    const version = tag ? tag[1] : "";

    if (commit) {
        if (!["identical", "behind"].includes(commit.status)) {
            reasons.push(`the commit is not on main (compare main...sha says '${commit.status}')`);
        }
        if (version) {
            const subject = new RegExp(`^(?:[\\w.-]+@[\\w.-]+: )?chore: release v${escapeRegex(version)}(?: \\(#\\d+\\))?$`);
            if (!subject.test(commit.subject ?? "")) {
                reasons.push(`the subject '${String(commit.subject).slice(0, 80)}' is not 'chore: release v${version}' (optionally with an '<agent>@<host>: ' prefix and a ' (#<number>)' suffix)`);
            }
        }
        const modified = new Set();
        for (const f of commit.files ?? []) {
            if (f.status === "modified" && VERSION_FILES.includes(f.filename)) {
                modified.add(f.filename);
            } else if (f.status === "removed" && /^\.changesets\/(?!README\.md$)[^/]+\.md$/.test(f.filename)) {
                // a consumed changeset
            } else {
                reasons.push(`the commit changes '${f.filename}' (${f.status}), which a release commit does not`);
            }
        }
        for (const name of VERSION_FILES) {
            if (!modified.has(name)) reasons.push(`the commit does not modify ${name}`);
        }
    } else {
        reasons.push("the commit could not be read");
    }

    if (version && versions) {
        for (const [name, v] of Object.entries(versions)) {
            if (v !== version) reasons.push(`${name} is at version '${v}', the tag is '${version}'`);
        }
    }
    return { ok: reasons.length === 0, version, reasons };
}

// ── the GitHub side ──────────────────────────────────────────────────────

function gh(args) {
    return execFileSync(process.env.GH_BIN || "gh", args, { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
}

/** compare status, subject and the changed files (paginated: a release deletes dozens of changesets). */
export function readCommit(repo, sha) {
    const status = gh(["api", `repos/${repo}/compare/main...${sha}`, "--jq", ".status"]).trim();
    const subject = gh(["api", `repos/${repo}/commits/${sha}`, "--jq", ".commit.message"]).split("\n", 1)[0];
    const files = gh(["api", "--paginate", `repos/${repo}/commits/${sha}?per_page=100`, "--jq", '.files[] | "\\(.status)\\t\\(.filename)"'])
        .split("\n")
        .filter(Boolean)
        .map((l) => {
            const [fileStatus, ...name] = l.split("\t");
            return { status: fileStatus, filename: name.join("\t") };
        });
    return { status, subject, files };
}

export function readVersions(repo, sha) {
    const texts = {};
    for (const name of VERSION_FILES) {
        texts[name] = gh(["api", "-H", "Accept: application/vnd.github.raw+json", `repos/${repo}/contents/${name}?ref=${sha}`]);
    }
    return parseVersions(texts);
}

function arg(argv, name) {
    const i = argv.indexOf(`--${name}`);
    return i >= 0 ? argv[i + 1] : undefined;
}

export function main(argv) {
    const refType = arg(argv, "ref-type");
    const refName = arg(argv, "ref-name");
    const sha = arg(argv, "sha");
    const repo = arg(argv, "repo");
    const out = arg(argv, "github-output");
    if (!repo || !/^[\w.-]+\/[\w.-]+$/.test(repo)) {
        console.error("::error::--repo owner/name is required");
        return 2;
    }
    // Refuse before touching the API when the shape alone is wrong.
    let commit;
    let versions;
    if (refType === "tag" && TAG_RE.test(refName ?? "") && SHA_RE.test(sha ?? "")) {
        try {
            commit = readCommit(repo, sha);
            versions = readVersions(repo, sha);
        } catch (e) {
            console.error(`::error::could not read the commit from GitHub: ${e.message.split("\n")[0]}`);
            return 1;
        }
    }
    const result = evaluateGate({ refType, refName, sha, commit, versions });
    if (!result.ok) {
        for (const r of result.reasons) console.error(`::error::release gate: ${r}`);
        console.error(`::error::refusing to build, sign or publish ${refName} at ${sha}`);
        return 1;
    }
    console.log(`release gate: ${refName} at ${sha} is the chore: release commit on main for ${result.version}`);
    if (out) appendFileSync(out, `version=${result.version}\nsha=${sha}\n`);
    return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
    process.exit(main(process.argv.slice(2)));
}
