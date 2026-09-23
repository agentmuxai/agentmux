// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for ci-required-check.mjs.
// Regression for #3506: a matrix job skipped by a job-level `if:` reports
// under its UNEXPANDED name ("check --tests + test (${{ matrix.os }})"), not
// the per-leg name branch protection is pinned to. That context then never
// arrives and the PR is permanently BLOCKED. This is the aggregate gate that
// replaces pinning branch protection to the matrix leg directly.

import { describe, expect, it } from "vitest";
import { evaluateRequiredJobs } from "./ci-required-check.mjs";

describe("evaluateRequiredJobs — the case #3506 exists to fix", () => {
    it("passes when a path-conditional job was legitimately skipped", () => {
        // Exactly #3505's shape: a docs-only PR skips rust/frontend via
        // SPEC_CI_PATH_CONDITIONAL_CHECKS_2026_09_21.md's classifier.
        const result = evaluateRequiredJobs({
            changes: { result: "success" },
            rust: { result: "skipped" },
            frontend: { result: "skipped" },
        });
        expect(result.ok).toBe(true);
        expect(result.failing).toEqual([]);
    });

    it("passes when every job actually ran and succeeded", () => {
        const result = evaluateRequiredJobs({
            changes: { result: "success" },
            rust: { result: "success" },
            frontend: { result: "success" },
        });
        expect(result.ok).toBe(true);
    });

    // A docs-only PR skips rust and frontend, so `docs` is the ONLY gate left
    // running — and it is not itself a branch-protection context. Until it was
    // added to this job's `needs:`, nothing consumed its result and a red doc
    // gate merged anyway. Observed live on #3510: a malformed spec Status
    // header — precisely the content error that gate exists to catch — with
    // `CI required` green and the PR approved twice.
    it("fails a docs-only PR whose doc gate failed", () => {
        const result = evaluateRequiredJobs({
            changes: { result: "success" },
            docs: { result: "failure" },
            rust: { result: "skipped" },
            frontend: { result: "skipped" },
        });
        expect(result.ok).toBe(false);
        expect(result.failing).toEqual(["docs"]);
    });

    it("passes a docs-only PR whose doc gate succeeded", () => {
        const result = evaluateRequiredJobs({
            changes: { result: "success" },
            docs: { result: "success" },
            rust: { result: "skipped" },
            frontend: { result: "skipped" },
        });
        expect(result.ok).toBe(true);
    });
});

describe("evaluateRequiredJobs — real failures must still block", () => {
    it("fails when a required job actually failed", () => {
        const result = evaluateRequiredJobs({
            changes: { result: "success" },
            rust: { result: "failure" },
            frontend: { result: "skipped" },
        });
        expect(result.ok).toBe(false);
        expect(result.failing).toEqual(["rust"]);
    });

    it("fails when a required job was cancelled", () => {
        // Not "skipped" -- a cancelled run (e.g. a timeout, or a superseded
        // run's own jobs before the newer push's run completes) must not be
        // indistinguishable from a legitimate path-conditional skip.
        const result = evaluateRequiredJobs({
            changes: { result: "success" },
            rust: { result: "cancelled" },
            frontend: { result: "success" },
        });
        expect(result.ok).toBe(false);
        expect(result.failing).toEqual(["rust"]);
    });

    it("names every failing job, not just the first", () => {
        const result = evaluateRequiredJobs({
            changes: { result: "success" },
            rust: { result: "failure" },
            frontend: { result: "cancelled" },
        });
        expect(result.ok).toBe(false);
        expect(result.failing.sort()).toEqual(["frontend", "rust"]);
    });
});

describe("evaluateRequiredJobs — fails toward blocking on malformed input", () => {
    // Mirrors ci-classify-changes.mjs's R2 posture one level up: this gate
    // existing to catch is exactly "the wrong answer doesn't fail loudly, it
    // reports green" -- an aggregate that itself fails open on a weird shape
    // would reproduce the class of bug it was built to close.
    it("fails on an empty needs object rather than vacuously passing", () => {
        const result = evaluateRequiredJobs({});
        expect(result.ok).toBe(false);
    });

    it("fails on null", () => {
        const result = evaluateRequiredJobs(null);
        expect(result.ok).toBe(false);
    });

    it("fails on undefined", () => {
        const result = evaluateRequiredJobs(undefined);
        expect(result.ok).toBe(false);
    });

    it("treats an unrecognized result value as failing, not passing", () => {
        // A future GitHub Actions result value this was never updated for
        // (or a typo in a test fixture) must not silently count as fine.
        const result = evaluateRequiredJobs({
            changes: { result: "success" },
            rust: { result: "some_future_value" },
        });
        expect(result.ok).toBe(false);
        expect(result.failing).toEqual(["rust"]);
    });

    it("treats a job entry missing its result field as failing", () => {
        const result = evaluateRequiredJobs({
            changes: { result: "success" },
            rust: {},
        });
        expect(result.ok).toBe(false);
        expect(result.failing).toEqual(["rust"]);
    });
});
