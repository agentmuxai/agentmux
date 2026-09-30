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
    SKIP_AUTHOR,
    skippedHead,
    TRIGGER_AUTHOR,
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

// #3562: ReAgent asked for 259331cc79 and Codex answered with its quota
// notice, which names no commit; the gate sat pending.
const trigger = (sha, at, login = TRIGGER_AUTHOR) => ({
    user: { login },
    created_at: at,
    body: `@codex review\n\n<!-- reagent:codex-trigger head=${sha} -->`,
});
const quotaComment = (at) => ({
    user: { login: CODEX_LOGIN },
    created_at: at,
    body:
        "You have reached your Codex usage limits for code reviews. You can see your limits in the " +
        "[Codex usage dashboard](https://chatgpt.com/codex/cloud/settings/usage).",
});

describe("Codex out of review quota", () => {
    it("passes the head the quota notice answered", () => {
        // #3562's real sequence: Codex answered the OLD trigger (14:36)
        // before ReAgent asked about HEAD.
        const comments = [
            trigger(OLD, "2026-09-23T14:31:42Z"),
            trigger(HEAD, "2026-09-23T14:46:38Z"),
            quotaComment("2026-09-23T14:46:48Z"),
        ];
        const reviews = [findingsReview(OLD, "2026-09-23T14:36:45Z")];
        const r = evaluateCodexGate({ headSha: HEAD, comments, reviews });
        expect(r.state).toBe("success");
        expect(r.description).toMatch(/quota/);
        expect(r.description.length).toBeLessThanOrEqual(140);
    });

    it("stays pending when two requests are outstanding (Codex P1 on #3589)", () => {
        // A late reply to the OLD request must not pass the unreviewed HEAD.
        const comments = [
            trigger(OLD, "2026-09-23T14:31:42Z"),
            trigger(HEAD, "2026-09-23T14:46:38Z"),
            quotaComment("2026-09-23T14:46:48Z"),
        ];
        expect(evaluateCodexGate({ headSha: HEAD, comments }).state).toBe("pending");
    });

    it("lets an answer close only its own request (Codex P1 on #3589, second)", () => {
        // findings(OLD) answers only OLD, so the quota notice answers HEAD alone.
        const comments = [
            trigger(OLD, "2026-09-23T14:31:42Z"),
            trigger(HEAD, "2026-09-23T14:32:00Z"),
            quotaComment("2026-09-23T14:40:00Z"),
        ];
        const reviews = [findingsReview(OLD, "2026-09-23T14:36:45Z")];
        expect(evaluateCodexGate({ headSha: HEAD, comments, reviews }).state).toBe("success");
    });

    it("resolves once the next request is answered alone", () => {
        const comments = [
            trigger(OLD, "2026-09-23T14:31:42Z"),
            trigger(HEAD, "2026-09-23T14:46:38Z"),
            quotaComment("2026-09-23T14:46:48Z"),
            trigger(HEAD, "2026-09-23T15:00:00Z"),
            quotaComment("2026-09-23T15:00:09Z"),
        ];
        expect(evaluateCodexGate({ headSha: HEAD, comments }).state).toBe("success");
    });

    it("does not pass a head pushed after the notice", () => {
        const comments = [trigger(OLD, "2026-09-23T14:31:42Z"), quotaComment("2026-09-23T14:31:50Z")];
        expect(evaluateCodexGate({ headSha: HEAD, comments }).state).toBe("pending");
    });

    it("ignores a trigger posted after the notice", () => {
        const comments = [
            trigger(OLD, "2026-09-23T14:31:42Z"),
            quotaComment("2026-09-23T14:31:50Z"),
            trigger(HEAD, "2026-09-23T14:46:38Z"),
        ];
        expect(evaluateCodexGate({ headSha: HEAD, comments }).state).toBe("pending");
    });

    it("ignores a trigger marker from anyone but ReAgent's a5af", () => {
        const comments = [trigger(HEAD, "2026-09-23T14:46:38Z", "someone"), quotaComment("2026-09-23T14:46:48Z")];
        expect(evaluateCodexGate({ headSha: HEAD, comments }).state).toBe("pending");
    });

    it("ignores a quota notice posted by anyone but Codex", () => {
        const fake = { ...quotaComment("2026-09-23T14:46:48Z"), user: { login: "someone" } };
        const comments = [trigger(HEAD, "2026-09-23T14:46:38Z"), fake];
        expect(evaluateCodexGate({ headSha: HEAD, comments }).state).toBe("pending");
    });

    it("lets a later real review of the same head win", () => {
        const comments = [trigger(HEAD, "2026-09-23T14:46:38Z"), quotaComment("2026-09-23T14:46:48Z")];
        const reviews = [findingsReview(HEAD, "2026-09-23T16:00:00Z")];
        expect(evaluateCodexGate({ headSha: HEAD, comments, reviews }).state).toBe("failure");
    });
});

// Mirrors reagent's codex_policy.is_docs_only_path: ReAgent doesn't re-ask
// Codex for a docs-only diff after an OK, so the gate must carry that OK.
describe("isDocsOnlyPath", () => {
    it("accepts the docs tree, changesets and README/CHANGELOG/LICENSE", () => {
        for (const p of ["docs/specs/SPEC_X.md", ".changesets/1-fix.md", ".changeset/brave-fox.md", "README.md", "src/README.md",
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

// ReAgent no longer re-asks Codex when only doc files it flagged changed
// (a5af/reagent spec codex-efficiency-2026-09-30 §3), so findings that only
// touch docs must not leave the PR failing or waiting.
const reviewWithId = (id, sha, at) => ({ ...findingsReview(sha, at), id });
const inline = (reviewId, path) => ({ pull_request_review_id: reviewId, path, user: { login: CODEX_LOGIN } });

describe("findings only on docs", () => {
    it("passes docs-only findings on the head", () => {
        const r = evaluateCodexGate({
            headSha: HEAD,
            reviews: [reviewWithId(11, HEAD)],
            reviewComments: [inline(11, "docs/specs/SPEC_X.md"), inline(11, "README.md")],
        });
        expect(r.state).toBe("success");
        expect(r.description).toBe(`Codex only flagged docs in ${HEAD.slice(0, 10)}; see its comments`);
        expect(r.description.length).toBeLessThanOrEqual(140);
    });

    const docsFindingsOnOld = { reviews: [reviewWithId(11, OLD)], reviewComments: [inline(11, "docs/specs/SPEC_X.md")] };

    it("carries docs-only findings to a later head when only docs changed since", () => {
        const r = evaluateCodexGate({ headSha: HEAD, ...docsFindingsOnOld, filesSinceLatest: ["docs/specs/SPEC_X.md"] });
        expect(r.state).toBe("success");
        expect(r.description).toContain(OLD.slice(0, 10));
    });

    it("waits for Codex when code changed since docs-only findings", () => {
        // ReAgent re-asks for a non-doc change, so that answer decides the head.
        const r = evaluateCodexGate({
            headSha: HEAD,
            ...docsFindingsOnOld,
            filesSinceLatest: ["docs/specs/SPEC_X.md", "src/lib.rs"],
        });
        expect(r.state).toBe("pending");
    });

    it("waits for Codex when the diff since docs-only findings is unknown", () => {
        expect(evaluateCodexGate({ headSha: HEAD, ...docsFindingsOnOld, filesSinceLatest: null }).state).toBe("pending");
        expect(evaluateCodexGate({ headSha: HEAD, ...docsFindingsOnOld }).state).toBe("pending");
    });

    it("still fails findings that mix code and docs", () => {
        const reviewComments = [inline(11, "docs/specs/SPEC_X.md"), inline(11, "src/lib.rs")];
        expect(evaluateCodexGate({ headSha: HEAD, reviews: [reviewWithId(11, HEAD)], reviewComments }).state).toBe(
            "failure",
        );
        const carried = { headSha: HEAD, reviews: [reviewWithId(11, OLD)], reviewComments, filesSinceLatest: ["docs/a.md"] };
        expect(evaluateCodexGate(carried).state).toBe("pending");
    });

    it("treats findings with no known files as before (fail safe)", () => {
        expect(evaluateCodexGate({ headSha: HEAD, reviews: [reviewWithId(11, HEAD)] }).state).toBe("failure");
        const carried = { headSha: HEAD, reviews: [reviewWithId(11, OLD)], filesSinceLatest: ["docs/a.md"] };
        expect(evaluateCodexGate(carried).state).toBe("pending");
    });

    it("only counts inline comments from that review", () => {
        // Docs-only comments from an older review don't clear a code finding.
        const r = evaluateCodexGate({
            headSha: HEAD,
            reviews: [reviewWithId(10, OLD, "2026-09-23T05:00:00Z"), reviewWithId(11, HEAD, "2026-09-23T06:00:00Z")],
            reviewComments: [inline(10, "docs/a.md"), inline(11, "src/lib.rs")],
        });
        expect(r.state).toBe("failure");
    });

    it("does not carry docs-only findings past a later code finding", () => {
        const r = evaluateCodexGate({
            headSha: HEAD,
            reviews: [
                reviewWithId(10, OLD, "2026-09-23T05:00:00Z"),
                reviewWithId(11, "aaaaaaaaaa11", "2026-09-23T06:00:00Z"),
            ],
            reviewComments: [inline(10, "docs/a.md"), inline(11, "src/lib.rs")],
            filesSinceLatest: ["docs/a.md"],
        });
        expect(r.state).toBe("pending");
    });
});

// While Codex's quota is exhausted ReAgent doesn't ask; it keeps one comment
// per PR naming the head it skipped (spec §2).
const skipComment = (sha, { login = SKIP_AUTHOR, created = "2026-09-30T10:00:00Z", updated } = {}) => ({
    user: { login },
    created_at: created,
    ...(updated ? { updated_at: updated } : {}),
    body:
        `Codex not asked about \`${sha.slice(0, 10)}\`: out of review quota until 11:00 UTC. Comment ` +
        "`@reagentx-workflow codex re-review` to try sooner.\n" +
        `<!-- reagent:codex-skipped reason=quota head=${sha} -->`,
});

describe("ReAgent quota skip marker", () => {
    it("passes the head ReAgent skipped", () => {
        const r = evaluateCodexGate({ headSha: HEAD, comments: [skipComment(HEAD)] });
        expect(r.state).toBe("success");
        expect(r.description).toBe(`Codex is out of review quota; ${HEAD.slice(0, 10)} passes without it`);
    });

    it("ignores the same marker from anyone else", () => {
        for (const login of ["someone", "a5af", CODEX_LOGIN, "reagentx-workflow"]) {
            const r = evaluateCodexGate({ headSha: HEAD, comments: [skipComment(HEAD, { login })] });
            expect(r.state).toBe("pending");
        }
    });

    it("follows the comment when ReAgent edits it to a newer head", () => {
        const edited = skipComment(HEAD, { created: "2026-09-30T10:00:00Z", updated: "2026-09-30T10:30:00Z" });
        expect(evaluateCodexGate({ headSha: HEAD, comments: [edited] }).state).toBe("success");
        // The edit replaced OLD's marker, so OLD is no longer passed by it.
        expect(evaluateCodexGate({ headSha: OLD, comments: [edited] }).state).toBe("pending");
    });

    // a5af/reagent#282: ReAgent writes the note before reading Codex's
    // answers, so a skip can name, and post-date, a head Codex answered.
    it("never overrides Codex findings on the head, even when the skip is newer", () => {
        const r = evaluateCodexGate({
            headSha: HEAD,
            reviews: [findingsReview(HEAD, "2026-09-30T10:10:00Z")],
            comments: [skipComment(HEAD, { created: "2026-09-30T10:00:00Z", updated: "2026-09-30T10:30:00Z" })],
        });
        expect(r.state).toBe("failure");
    });

    it("leaves a Codex OK on the head as the reason it passes", () => {
        const r = evaluateCodexGate({
            headSha: HEAD,
            comments: [okComment(HEAD, "2026-09-30T10:10:00Z"), skipComment(HEAD, { updated: "2026-09-30T10:30:00Z" })],
        });
        expect(r.state).toBe("success");
        expect(r.description).toBe(`Codex found no major issues in ${HEAD.slice(0, 10)}`);
    });

    it("is not a verdict to carry forward", () => {
        // An OK on OLD, then a skip for OLD: the OK still carries across docs.
        const comments = [okComment(OLD, "2026-09-30T10:00:00Z"), skipComment(OLD, { updated: "2026-09-30T10:30:00Z" })];
        expect(latestCodexOutput({ comments })).toMatchObject({ kind: "ok" });
        const r = evaluateCodexGate({ headSha: HEAD, comments, filesSinceLatest: ["docs/a.md"] });
        expect(r.description).toBe(`Codex OK on ${OLD.slice(0, 10)}; only docs changed since`);
        // A skip alone on OLD passes nothing on HEAD.
        expect(
            evaluateCodexGate({ headSha: HEAD, comments: [skipComment(OLD)], filesSinceLatest: ["docs/a.md"] }).state,
        ).toBe("pending");
    });

    it("lets a later real review of the head win", () => {
        const r = evaluateCodexGate({
            headSha: HEAD,
            comments: [skipComment(HEAD)],
            reviews: [findingsReview(HEAD, "2026-09-30T12:00:00Z")],
        });
        expect(r.state).toBe("failure");
    });

    it("does not pass a head other than the one named", () => {
        expect(evaluateCodexGate({ headSha: HEAD, comments: [skipComment(OLD)] }).state).toBe("pending");
    });

    it("reads a short or upper-case sha and rejects a malformed one", () => {
        const c = skipComment(HEAD);
        expect(skippedHead({ ...c, body: c.body.replace(HEAD, HEAD.slice(0, 7).toUpperCase()) })).toBe(HEAD.slice(0, 7));
        expect(skippedHead({ ...c, body: c.body.replace(HEAD, "abc12") })).toBeNull();
        expect(skippedHead({ ...c, body: c.body.replace("reason=quota", "reason=other") })).toBeNull();
    });
});
