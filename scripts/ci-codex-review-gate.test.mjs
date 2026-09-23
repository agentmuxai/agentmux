// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for ci-codex-review-gate.mjs. The gate's one job is to never go
// green for a commit Codex has not OK'd, so most cases here are ways it
// could wrongly succeed.

import { describe, expect, it } from "vitest";
import { CODEX_LOGIN, evaluateCodexGate, reviewedCommit } from "./ci-codex-review-gate.mjs";

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
});
