// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A job that sparse-checks out a script must also check out every module the
// script imports. A missing one passes every test and CI, and then crashes the
// job on every PR once merged: for a job that runs main's copy (the Codex
// review gate), every PR in the repo is blocked. This walks each workflow's
// jobs and fails on any `node scripts/….mjs` whose relative imports, followed
// transitively, its job's sparse list doesn't cover.
//
// Jobs are found by their two-space key under the top-level `jobs:`, which
// every workflow here uses; the last test fails if that ever stops matching.

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, posix } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const WORKFLOWS = join(ROOT, ".github/workflows");

/** { name, text } for each job in a workflow file. */
function jobsOf(source) {
    const lines = source.replace(/\r\n/g, "\n").split("\n");
    const start = lines.findIndex((l) => /^jobs:\s*$/.test(l));
    if (start < 0) return [];
    const jobs = [];
    for (const line of lines.slice(start + 1)) {
        if (/^\S/.test(line) && !line.startsWith("#")) break; // the next top-level key
        const key = /^ {2}([A-Za-z0-9_-]+):\s*$/.exec(line);
        if (key) jobs.push({ name: key[1], lines: [] });
        else jobs.at(-1)?.lines.push(line);
    }
    return jobs.map(({ name, lines }) => ({ name, text: lines.join("\n") }));
}

/** Every path a job's `sparse-checkout:` keys list (inline or as a block). */
function sparsePaths(jobText) {
    const lines = jobText.split("\n");
    const paths = [];
    lines.forEach((line, i) => {
        const m = /^(\s*)sparse-checkout:\s*(.*?)\s*$/.exec(line);
        if (!m) return;
        if (m[2] !== "|") return void paths.push(m[2]);
        for (const next of lines.slice(i + 1)) {
            if (next.trim() === "" || next.search(/\S/) <= m[1].length) break;
            paths.push(next.trim());
        }
    });
    return paths;
}

/** Repo-relative paths of `entry` and every module it imports, transitively. */
function moduleClosure(entry, seen = new Set()) {
    if (seen.has(entry)) return seen;
    seen.add(entry);
    const source = readFileSync(join(ROOT, entry), "utf8");
    for (const [, spec] of source.matchAll(/^\s*(?:import|export)\b[^;]*?\bfrom\s+["'](\.{1,2}\/[^"']+)["']/gm)) {
        moduleClosure(posix.normalize(posix.join(posix.dirname(entry), spec)), seen);
    }
    for (const [, spec] of source.matchAll(/^\s*import\s+["'](\.{1,2}\/[^"']+)["']/gm)) {
        moduleClosure(posix.normalize(posix.join(posix.dirname(entry), spec)), seen);
    }
    return seen;
}

/** A sparse entry covers a file it names, or any file under a directory it names. */
const covers = (paths, file) => paths.some((p) => file === p || file.startsWith(p.replace(/\/$/, "") + "/"));

const sparseJobs = readdirSync(WORKFLOWS)
    .filter((f) => /\.ya?ml$/.test(f))
    .flatMap((file) =>
        jobsOf(readFileSync(join(WORKFLOWS, file), "utf8")).map((job) => ({
            id: `${file} ${job.name}`,
            paths: sparsePaths(job.text),
            scripts: [...new Set([...job.text.matchAll(/\bnode\s+(?:\.\/)?(scripts\/[\w./-]+\.mjs)\b/g)].map((m) => m[1]))],
        })),
    )
    .filter((job) => job.paths.length > 0 && job.scripts.length > 0);

describe("sparse checkouts cover what their scripts import", () => {
    it.each(sparseJobs.map((job) => [job.id, job]))("%s", (_id, job) => {
        for (const script of job.scripts) {
            const missing = [...moduleClosure(script)].filter((file) => !covers(job.paths, file));
            expect(missing, `${script} needs these in the job's sparse-checkout`).toEqual([]);
        }
    });

    it("finds the jobs it is meant to guard", () => {
        expect(sparseJobs.map((j) => j.id)).toEqual(
            expect.arrayContaining([
                "codex-review-gate.yml evaluate",
                "codex-review-gate.yml sweep",
                "ci-pr.yml changes",
                "ci-pr.yml ci-required",
                "srv-image.yml changes",
            ]),
        );
    });
});
