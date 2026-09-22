#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Decide whether the aggregate "CI required" gate should pass, given the
 * `result` of each job it depends on.
 *
 * Exists because a matrix job skipped by a job-level `if:` is reported to
 * GitHub under its UNEXPANDED name (e.g. `"check --tests + test (${{
 * matrix.os }})"`), never the per-leg name (`"check --tests + test
 * (windows-latest)"`) that branch protection was pinned to. That context can
 * then never arrive for a PR whose classifier legitimately skips the leg —
 * the PR is permanently BLOCKED regardless of review or re-runs, no matter
 * how many times CI reruns. Observed live on #3505 (a docs-only PR),
 * introduced by #3491, filed as #3506.
 *
 * The fix: branch protection points at ONE job (this script's caller in
 * ci-pr.yml) instead of the matrix leg and vitest directly. That job
 * `needs:` every job branch protection used to pin individually, runs
 * unconditionally (`if: always()`), and this script inspects each
 * dependency's own `result` explicitly — NOT `needs.*.result` implicitly via
 * GitHub's default `needs`-success gating, which would fail this job (and so
 * the whole PR) on every legitimately SKIPPED path-conditional leg,
 * reproducing the exact bug this exists to close, one level up.
 *
 * A job's `result` is one of: success, failure, cancelled, skipped.
 * `skipped` is fine here — it means
 * SPEC_CI_PATH_CONDITIONAL_CHECKS_2026_09_21.md's classifier decided this PR
 * didn't need that job, which is the entire point of that spec. `failure`
 * and `cancelled` are not. Anything else — a typo'd job name, a future
 * GitHub Actions result value this was never updated for — fails CLOSED:
 * the same "fail toward running" posture ci-classify-changes.mjs's R2
 * documents, one level up. An aggregate gate that fails OPEN on a weird
 * value would itself become the next version of this bug.
 *
 * Usage (see ci-pr.yml):
 *   NEEDS_JSON=$(printf '%s' "${{ toJSON(needs) }}") node ci-required-check.mjs
 */

const OK_RESULTS = new Set(["success", "skipped"]);

/**
 * @param {unknown} needs - the GitHub Actions `needs` context, e.g.
 *   `{"rust": {"result": "skipped"}, "frontend": {"result": "success"}}`.
 * @returns {{ok: boolean, failing: string[]}}
 */
export function evaluateRequiredJobs(needs) {
    if (needs === null || typeof needs !== "object") {
        return { ok: false, failing: ["<no jobs reported>"] };
    }
    const names = Object.keys(needs);
    if (names.length === 0) {
        // Fail closed rather than vacuously pass — an empty `needs` context
        // here means something upstream is broken (a bad `toJSON(needs)`
        // expression, a `needs:` list edited down to nothing), not that
        // there was genuinely nothing to check.
        return { ok: false, failing: ["<no jobs reported>"] };
    }
    const failing = names.filter((name) => {
        const entry = needs[name];
        const result = entry && typeof entry === "object" ? entry.result : undefined;
        return !OK_RESULTS.has(result);
    });
    return { ok: failing.length === 0, failing };
}

async function readStdin() {
    if (process.stdin.isTTY) return "";
    let data = "";
    for await (const chunk of process.stdin) data += chunk;
    return data;
}

// Entry point only when executed directly, so the test can import
// evaluateRequiredJobs without this running.
const isMain = process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/\\/g, "/").split("/").pop());
if (isMain) {
    const raw = process.env.NEEDS_JSON ?? (await readStdin());
    let needs;
    try {
        needs = JSON.parse(raw);
    } catch (e) {
        console.error(`ci-required-check: could not parse needs JSON: ${e.message}`);
        console.error(`ci-required-check: raw input was: ${raw}`);
        process.exit(1);
    }
    const { ok, failing } = evaluateRequiredJobs(needs);
    if (ok) {
        console.log(`ci-required-check: all ${Object.keys(needs).length} job(s) succeeded or were legitimately skipped`);
        process.exit(0);
    }
    console.error(`ci-required-check: FAILED — not success/skipped: ${failing.join(", ")}`);
    for (const name of failing) {
        console.error(`  ${name}: ${JSON.stringify(needs[name])}`);
    }
    process.exit(1);
}
