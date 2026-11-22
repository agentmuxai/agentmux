// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The files this branch changes, the way every changed-files gate must see them.
//
// A gate that asks "which files did this PR change?" compares the merge-base with the PR's base against the
// WORKING TREE, so a change that is staged or edited counts the same as one that is committed. Against HEAD alone, a
// spec that was written and `git add`ed but not committed was invisible: the docs gates said "no specs changed" before
// the commit and failed after it. In CI the working tree is HEAD, so the answer there is the same as
// `git diff <base>...HEAD`.
//
// A file git does not track is not on the branch until it is added (an agent keeps a spec it has not shipped yet
// untracked on purpose), so it is left out unless `untracked: true` asks for it, as `A`.
//
//   node scripts/lib/branch-changes.mjs [--ref origin/main] [--added] [--untracked]
//
// prints git's `--name-status` rows (tab-separated; a rename has both paths; deleted files left out) so a shell gate
// can read them as it read `git diff --name-status --find-renames`. With `--added` it prints only the paths of added files.
import { execFileSync } from "node:child_process";
import { isMain } from "./is-main.mjs";

function git(root, args) {
    return execFileSync("git", args, { cwd: root, encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
}

function tryGit(root, args) {
    try {
        return git(root, args);
    } catch {
        return null;
    }
}

/** The PR's base ref: `origin/<base>` when it exists, else `<base>` (`$GITHUB_BASE_REF`, default `main`). Null when neither resolves. */
export function baseRef(root = process.cwd(), env = process.env) {
    const base = env.GITHUB_BASE_REF || "main";
    for (const ref of [`origin/${base}`, base]) {
        if (tryGit(root, ["rev-parse", "--verify", "--quiet", ref]) !== null) return ref;
    }
    return null;
}

/**
 * Rows of `[status, path]` (`[status, from, to]` for a rename or copy) for every file the branch changes against `ref`:
 * committed, staged and edited, plus the untracked ones (as `A`) when `untracked` is set. Null when there is no
 * merge base to compare with.
 */
export function changedRows({ root = process.cwd(), ref = baseRef(root), untracked = false } = {}) {
    if (!ref) return null;
    const mergeBase = tryGit(root, ["merge-base", ref, "HEAD"]);
    if (mergeBase === null) return null;
    const diff = git(root, ["diff", "--name-status", "-z", "--find-renames", mergeBase.trim()]);
    const rows = [];
    const fields = diff.split("\0");
    for (let i = 0; i < fields.length && fields[i] !== ""; ) {
        const status = fields[i++];
        const paths = status[0] === "R" || status[0] === "C" ? 2 : 1;
        rows.push([status, ...fields.slice(i, i + paths)]);
        i += paths;
    }
    if (untracked) for (const path of untrackedFiles(root)) rows.push(["A", path]);
    return rows;
}

/** Files git does not track and does not ignore. */
export function untrackedFiles(root = process.cwd()) {
    return git(root, ["ls-files", "-z", "--others", "--exclude-standard"]).split("\0").filter(Boolean);
}

/** The destination path of each row whose status letter is in `letters` (default: every row but a deletion). */
export function pathsOf(rows, letters = "ACMRTU") {
    return (rows ?? []).filter((r) => letters.includes(r[0][0])).map((r) => r[r.length - 1]);
}

if (isMain(import.meta.url)) {
    const args = process.argv.slice(2);
    const at = args.indexOf("--ref");
    const rows = changedRows({ ...(at >= 0 ? { ref: args[at + 1] } : {}), untracked: args.includes("--untracked") });
    if (rows === null) {
        console.error("branch-changes: no base to compare with; nothing to check.");
    } else if (args.includes("--added")) {
        for (const p of pathsOf(rows, "A")) console.log(p);
    } else {
        for (const r of rows.filter((x) => x[0][0] !== "D")) console.log(r.join("\t"));
    }
}
