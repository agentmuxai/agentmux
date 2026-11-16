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
//        [--suite-proof <empty|later|a job result> | --suite-result <result> | --suite-checked-later]
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
//   5. the five version locations at that commit all equal the tag's version;
//   6. the full test suite has passed on exactly that commit: the push run of CI (PR) on main
//      for it has finished green, or, when the caller is a job of release-core.yml that
//      `needs` the release's own suite job, that job's result is `success` (`--suite-result`).
// Why 4: a release commit then carries no code of its own. Its tree is its
// parent's (already on main, so merged through the merge queue with a review
// and the required checks) plus the version bump.
// Why 6: the merge queue runs only part of the test suite; the full suite runs on
// the push to main. Two PRs that were each green broke it on 2026-11-14 and the
// release commit built on top of that was tagged and released with its own push run
// red (0.60.28). Only the commit's own proof counts: a later commit's green run says
// nothing about a red release commit.
// Why the release's own job too: the merge queue can land a release commit in the same push as
// another pull request, and then the push run exists only for the push's last commit, so the
// release commit has no push run and never will (0.60.30). The release therefore runs the suite
// itself on the tagged commit, and nothing leaves the run until that job is green.
// Without a flag, check 6 is the push run, as before: a workflow that can also be started on
// its own for a tag gets no flag there. `--suite-result` is the literal `needs.suite.result`
// and only `success` passes (skipped, cancelled, failure and an empty value refuse, whatever
// the push run says). `--suite-checked-later` skips check 6 and says so, for a call that runs
// beside the suite and only builds inside the run; the jobs that send anything out of the run
// need the suite themselves. Giving both is refused.
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

/** What a job's result can be, as GitHub's `needs.<job>.result`; anything else given to `--suite-result` is a usage error. */
export const JOB_RESULTS = ["success", "failure", "cancelled", "skipped"];

/**
 * Pure decision. `commit` is what the GitHub API says about the commit:
 * `{ status: <compare main...sha>, subject, files: [{ status, filename }] }`;
 * `versions` is parseVersions() of the five files at the commit;
 * `pushRun` is the newest push run of CI (PR) on main for that commit, `{ status, conclusion, url }`,
 * or `null` when there is none, or `undefined` when it could not be read (refused too).
 * `suiteResult` is the result of the release's own suite job when the caller has one: then only `success`
 * passes and the push run is not asked. `suiteCheckedLater` leaves check 6 to the jobs that need the suite.
 * Returns { ok, version, reasons }.
 */
export function evaluateGate({ refType, refName, sha, commit, versions, pushRun, suiteResult, suiteCheckedLater }) {
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
    if (suiteResult !== undefined) {
        if (suiteResult !== "success") {
            reasons.push(`the release's own test suite finished '${suiteResult || "with no result"}' on this commit: it must be green first`);
        }
    } else if (!suiteCheckedLater) {
        const pushReason = pushRunReason(pushRun);
        if (pushReason) reasons.push(pushReason);
    }
    return { ok: reasons.length === 0, version, reasons };
}

/** Why a commit's push run does not allow a release, or "" when it finished green. */
export function pushRunReason(pushRun) {
    const where = "the push run of CI (PR) on main for this commit";
    if (pushRun === undefined) return `${where} could not be read`;
    if (pushRun === null) return `${where} was not found: it must finish green first`;
    if (pushRun.status !== "completed") return `${where} has not finished (${pushRun.status || "unknown"}): run this workflow again when it is green`;
    if (pushRun.conclusion !== "success") {
        return `${where} finished with '${pushRun.conclusion || "no result"}'${pushRun.url ? ` (${pushRun.url})` : ""}: it must be green first`;
    }
    return "";
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

/**
 * The push run of CI (PR) on main for exactly `sha` out of a list of workflow runs: `{ status, conclusion, url }`, or null
 * when none is for that commit and event. A re-run keeps the run's id and raises run_attempt, so the newest attempt is the
 * one read. A run of another commit, or of a pull request, never counts.
 */
export function pickPushRun(runs, sha) {
    const mine = (runs ?? []).filter((r) => r.head_sha === sha && r.event === "push");
    if (mine.length === 0) return null;
    mine.sort((a, b) => (a.run_number ?? 0) - (b.run_number ?? 0) || (a.run_attempt ?? 0) - (b.run_attempt ?? 0));
    const r = mine[mine.length - 1];
    return { status: r.status, conclusion: r.conclusion ?? null, url: r.html_url };
}

/**
 * Reads the runs through the run's token, and without one when that is refused: the repository is public and its workflow
 * runs are public data, so the release job needs no `actions: read` (the caller, release.yml, keeps the copy of its
 * permissions a tag was made with, and a called workflow cannot ask for more than its caller grants).
 */
export function readPushRun(repo, sha) {
    const query = `repos/${repo}/actions/workflows/ci-pr.yml/runs?head_sha=${sha}&event=push&branch=main&per_page=20`;
    let text;
    try {
        text = gh(["api", query]);
    } catch {
        text = execFileSync("curl", ["-fsS", "-H", "Accept: application/vnd.github+json", `https://api.github.com/${query}`], {
            encoding: "utf8",
            maxBuffer: 64 * 1024 * 1024,
        });
    }
    return pickPushRun(JSON.parse(text).workflow_runs, sha);
}

/**
 * `--suite-proof <value>` is how a reusable workflow passes what release-core.yml told it, through one input: an empty
 * value is no proof (check 6 is the push run, as for a run outside release-core), `later` is --suite-checked-later,
 * and anything else is --suite-result. null when the option is absent.
 */
export function suiteProofArgs(value) {
    if (value === undefined) return null;
    if (value === "") return { suiteResult: undefined, suiteCheckedLater: false };
    if (value === "later") return { suiteResult: undefined, suiteCheckedLater: true };
    return { suiteResult: value, suiteCheckedLater: false };
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
    const proof = suiteProofArgs(arg(argv, "suite-proof"));
    let suiteResult = arg(argv, "suite-result");
    let suiteCheckedLater = argv.includes("--suite-checked-later");
    if (proof) {
        if (suiteResult !== undefined || suiteCheckedLater) {
            console.error("::error::release gate: --suite-proof cannot be given with --suite-result or --suite-checked-later");
            return 2;
        }
        ({ suiteResult, suiteCheckedLater } = proof);
    }
    if (suiteResult !== undefined && !JOB_RESULTS.includes(suiteResult)) {
        console.error(`::error::release gate: --suite-result must be one of ${JOB_RESULTS.join(", ")}, not '${suiteResult}'`);
        return 2;
    }
    if (suiteResult !== undefined && suiteCheckedLater) {
        console.error("::error::release gate: --suite-result and --suite-checked-later cannot be given together");
        return 2;
    }
    if (!repo || !/^[\w.-]+\/[\w.-]+$/.test(repo)) {
        console.error("::error::--repo owner/name is required");
        return 2;
    }
    // Refuse before touching the API when the shape alone is wrong.
    let commit;
    let versions;
    let pushRun; // undefined stays "could not be read", which is refused
    if (refType === "tag" && TAG_RE.test(refName ?? "") && SHA_RE.test(sha ?? "")) {
        try {
            commit = readCommit(repo, sha);
            versions = readVersions(repo, sha);
        } catch (e) {
            console.error(`::error::could not read the commit from GitHub: ${e.message.split("\n")[0]}`);
            return 1;
        }
        // The push run is only asked for when nothing else carries check 6.
        if (suiteResult === undefined && !suiteCheckedLater) {
            try {
                pushRun = readPushRun(repo, sha);
            } catch (e) {
                console.error(`::warning::could not read the push run: ${e.message.split("\n")[0]}`);
            }
        }
    }
    const result = evaluateGate({ refType, refName, sha, commit, versions, pushRun, suiteResult, suiteCheckedLater });
    if (!result.ok) {
        for (const r of result.reasons) console.error(`::error::release gate: ${r}`);
        console.error(`::error::refusing to build, sign or publish ${refName} at ${sha}`);
        return 1;
    }
    console.log(`release gate: ${refName} at ${sha} is the chore: release commit on main for ${result.version}`);
    if (suiteCheckedLater) console.log("release gate: the test suite is checked later, by the jobs that need it");
    if (out) appendFileSync(out, `version=${result.version}\nsha=${sha}\n`);
    return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
    process.exit(main(process.argv.slice(2)));
}
