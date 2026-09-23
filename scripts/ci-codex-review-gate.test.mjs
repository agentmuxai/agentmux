// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for ci-codex-review-gate.mjs. The gate's one job is to never go
// green for a commit Codex has not OK'd, so most cases here are ways it
// could wrongly succeed.

import { describe, expect, it } from "vitest";
import {
    CODEX_LOGIN,
    evaluateCodexGate,
    isDocsOnlyPath,
    latestCodexOutput,
    reviewedCommit,
} from "./ci-codex-review-gate.mjs";

const HEAD = "4dba2e3755aa00000000000000000000000000bb";
const OLD = "57c2f24ee2cc00000000000000000000000000dd";

// Bodies trimmed from real Codex output on #3523.
const okComment = (sha, at = "2026-09-23T02:26:14Z", login = CODEX_LOGIN) => ({
    user: { login },
    created_at: at,
    body: `Codex Review: Didn't find any major issues. Keep it up!\n\n**Reviewed commit:** \`${sha.slice(0, 10)}\``,
});
const findingsReview = (sha, at = "2026-09-23T05:33:47Z") => ({
    user: { login: CODEX_LOGIN },
    submitted_at: at,
    body: `### 💡 Codex Review\n\nHere are some automated review suggestions for this pull request.\n\n**Reviewed commit:** \`${sha.slice(0, 10)}\``,
});

describe("reviewedCommit", () => {
    it("reads the sha Codex prints", () => {
        expect(reviewedCommit(okComment(HEAD).body)).toBe(HEAD.slice(0, 10));
    });
    it("returns null when there is no Reviewed commit line", () => {
        expect(reviewedCommit("Codex Review: Didn't find any major issues.")).toBeNull();
    });
});

describe("evaluateCodexGate", () => {
    it("succeeds on a Codex OK for the head commit", () => {
        const r = evaluateCodexGate({ headSha: HEAD, comments: [okComment(HEAD)] });
        expect(r.state).toBe("success");
    });

    it("accepts a curly apostrophe in the OK", () => {
        const c = okComment(HEAD);
        c.body = c.body.replace("Didn't", "Didn’t");
        expect(evaluateCodexGate({ headSha: HEAD, comments: [c] }).state).toBe("success");
    });

    it("stays pending with no Codex output at all", () => {
        expect(evaluateCodexGate({ headSha: HEAD }).state).toBe("pending");
    });

    it("does not carry an OK for an older commit over to a new push (#3513)", () => {
        const r = evaluateCodexGate({ headSha: HEAD, comments: [okComment(OLD)] });
        expect(r.state).toBe("pending");
    });

    it("ignores an OK posted by anyone but Codex", () => {
        const r = evaluateCodexGate({ headSha: HEAD, comments: [okComment(HEAD, undefined, "a5af")] });
        expect(r.state).toBe("pending");
    });

    it("ignores an OK with no Reviewed commit line", () => {
        const c = { user: { login: CODEX_LOGIN }, created_at: "2026-09-23T02:26:14Z", body: "Codex Review: Didn't find any major issues." };
        expect(evaluateCodexGate({ headSha: HEAD, comments: [c] }).state).toBe("pending");
    });

    it("fails when Codex's latest word on the head is a findings review", () => {
        const r = evaluateCodexGate({ headSha: HEAD, reviews: [findingsReview(HEAD)] });
        expect(r.state).toBe("failure");
    });

    it("lets a later OK on the same head clear earlier findings", () => {
        const r = evaluateCodexGate({
            headSha: HEAD,
            reviews: [findingsReview(HEAD, "2026-09-23T05:00:00Z")],
            comments: [okComment(HEAD, "2026-09-23T06:00:00Z")],
        });
        expect(r.state).toBe("success");
    });

    it("lets spontaneous findings after an OK take the OK back", () => {
        const r = evaluateCodexGate({
            headSha: HEAD,
            comments: [okComment(HEAD, "2026-09-23T05:00:00Z")],
            reviews: [findingsReview(HEAD, "2026-09-23T06:00:00Z")],
        });
        expect(r.state).toBe("failure");
    });

    it("drops a dismissed findings review back to pending, not success", () => {
        // Dismissal clears the failure, but only a Codex OK passes the gate.
        const r = evaluateCodexGate({
            headSha: HEAD,
            reviews: [{ ...findingsReview(HEAD), state: "DISMISSED" }],
        });
        expect(r.state).toBe("pending");
    });

    it("falls back to an earlier OK once later findings are dismissed", () => {
        const r = evaluateCodexGate({
            headSha: HEAD,
            comments: [okComment(HEAD, "2026-09-23T05:00:00Z")],
            reviews: [{ ...findingsReview(HEAD, "2026-09-23T06:00:00Z"), state: "DISMISSED" }],
        });
        expect(r.state).toBe("success");
    });

    it("ignores findings on an older commit", () => {
        const r = evaluateCodexGate({
            headSha: HEAD,
            reviews: [findingsReview(OLD, "2026-09-23T07:00:00Z")],
            comments: [okComment(HEAD, "2026-09-23T06:00:00Z")],
        });
        expect(r.state).toBe("success");
    });

    it("keeps descriptions within GitHub's 140-char status limit", () => {
        for (const r of [
            evaluateCodexGate({ headSha: HEAD }),
            evaluateCodexGate({ headSha: HEAD, comments: [okComment(HEAD)] }),
            evaluateCodexGate({ headSha: HEAD, reviews: [findingsReview(HEAD)] }),
        ]) {
            expect(r.description.length).toBeLessThanOrEqual(140);
        }
    });

    it("tells the agent how to get a review when waiting", () => {
        const r = evaluateCodexGate({ headSha: HEAD });
        expect(r.description).toMatch(/codex re-review/);
    });
});

// Mirrors reagent's codex_policy.is_docs_only_path: ReAgent doesn't re-ask
// Codex for a docs-only diff after an OK, so the gate must carry that OK.
describe("isDocsOnlyPath", () => {
    it("accepts the docs tree, changesets and README/CHANGELOG/LICENSE", () => {
        for (const p of ["docs/specs/SPEC_X.md", ".changesets/1-fix.md", "README.md", "src/README.md",
            "CHANGELOG.md", "LICENSE", "NOTICE"]) {
            expect(isDocsOnlyPath(p)).toBe(true);
        }
    });
    it("rejects markdown that agents read, and CI config", () => {
        for (const p of ["CLAUDE.md", "AGENTS.md", "prompts/review.md", ".github/workflows/README.md",
            "scripts/README.md", "src/lib.rs"]) {
            expect(isDocsOnlyPath(p)).toBe(false);
        }
    });
});

describe("latestCodexOutput", () => {
    it("returns Codex's most recent verdict on any commit", () => {
        const out = latestCodexOutput({ comments: [okComment(OLD)], reviews: [] });
        expect(out).toMatchObject({ kind: "ok", sha: OLD.slice(0, 10) });
    });
    it("ignores dismissed reviews", () => {
        expect(latestCodexOutput({ reviews: [{ ...findingsReview(OLD), state: "DISMISSED" }] })).toBeNull();
    });
});

describe("carrying an OK across a docs-only diff", () => {
    it("succeeds when only docs changed since Codex's OK", () => {
        const r = evaluateCodexGate({
            headSha: HEAD,
            comments: [okComment(OLD)],
            filesSinceLatest: ["docs/notes.md", "README.md"],
        });
        expect(r.state).toBe("success");
        expect(r.description).toContain(OLD.slice(0, 10));
    });

    it("stays pending when code changed since the OK", () => {
        const r = evaluateCodexGate({
            headSha: HEAD,
            comments: [okComment(OLD)],
            filesSinceLatest: ["docs/notes.md", "src/lib.rs"],
        });
        expect(r.state).toBe("pending");
    });

    it("stays pending when the diff is unknown", () => {
        const r = evaluateCodexGate({ headSha: HEAD, comments: [okComment(OLD)], filesSinceLatest: null });
        expect(r.state).toBe("pending");
    });

    it("never carries findings forward as an OK", () => {
        const r = evaluateCodexGate({
            headSha: HEAD,
            reviews: [findingsReview(OLD)],
            filesSinceLatest: ["docs/notes.md"],
        });
        expect(r.state).toBe("pending");
    });

    it("does not carry an OK that later findings superseded", () => {
        const r = evaluateCodexGate({
            headSha: HEAD,
            comments: [okComment(OLD, "2026-09-23T05:00:00Z")],
            reviews: [findingsReview(OLD, "2026-09-23T06:00:00Z")],
            filesSinceLatest: ["docs/notes.md"],
        });
        expect(r.state).toBe("pending");
    });
});
