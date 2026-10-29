// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for ci-codex-review-gate.mjs. The gate's one job is to never go
// green for a commit Codex has not OK'd, so most cases here are ways it
// could wrongly succeed.

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { createGitHubClient, GitHubError } from "./lib/github-rest.mjs";
import {
    AGE_REFRESH_MS,
    ageText,
    changedFiles,
    CODEX_LOGIN,
    evaluateCodexGate,
    isDependencyBotPr,
    isDocsOnlyPath,
    latestCodexOutput,
    needsDiffSince,
    prNoCode,
    reviewedCommit,
    SKIP_AUTHORS,
    skipClaim,
    skipNote,
    checkSkipClaim,
    STALE_REQUEST_MS,
    sweep,
    sweepReason,
    TRIGGER_AUTHOR,
    unansweredRequest,
    WORKFLOW_FILE,
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

// #3562: muxreview asked for 259331cc79 and Codex answered with its quota
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
        // before muxreview asked about HEAD.
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

    it("passes the head when it was asked twice and one notice answered both", () => {
        // An automatic ask, then a manual re-review of the same head, then one
        // usage-limit notice: it is the same request, so the notice answers it.
        const comments = [
            trigger(HEAD, "2026-10-25T02:23:56Z"),
            trigger(HEAD, "2026-10-25T13:38:17Z"),
            quotaComment("2026-10-25T13:38:33Z"),
        ];
        const r = evaluateCodexGate({ headSha: HEAD, comments });
        expect(r.state).toBe("success");
        expect(r.description).toMatch(/quota/);
    });

    it("takes an abbreviated repeat of the head for the same request", () => {
        const comments = [
            trigger(HEAD, "2026-10-25T02:23:56Z"),
            trigger(HEAD.slice(0, 10), "2026-10-25T13:38:17Z"),
            quotaComment("2026-10-25T13:38:33Z"),
        ];
        expect(evaluateCodexGate({ headSha: HEAD, comments }).state).toBe("success");
    });

    it("still stays pending when another head was asked too", () => {
        const comments = [
            trigger(OLD, "2026-10-25T02:23:56Z"),
            trigger(HEAD, "2026-10-25T02:30:00Z"),
            trigger(HEAD, "2026-10-25T13:38:17Z"),
            quotaComment("2026-10-25T13:38:33Z"),
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

    it("ignores a trigger marker from anyone but muxreview's a5af", () => {
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

// Mirrors reagent's codex_policy.is_docs_only_path: muxreview doesn't re-ask
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
        // Only docs since: nothing will ask Codex again, so the fix is owed.
        const r = evaluateCodexGate({
            headSha: HEAD,
            reviews: [findingsReview(OLD)],
            filesSinceLatest: ["docs/notes.md"],
        });
        expect(r.state).toBe("failure");
        expect(r.description).toContain(OLD.slice(0, 10));
        expect(r.description).toMatch(/codex re-review/);
        expect(r.description.length).toBeLessThanOrEqual(140);
    });

    it("waits, naming the findings, when code changed since findings on an earlier head", () => {
        // muxreview asks about any code change once it approves, flagged file
        // or not.
        const r = evaluateCodexGate({
            headSha: HEAD,
            reviews: [findingsReview(OLD)],
            filesSinceLatest: ["src/caller.rs"],
        });
        expect(r.state).toBe("pending");
        expect(r.description).toContain(OLD.slice(0, 10));
        expect(r.description).toContain(HEAD.slice(0, 10));
        expect(r.description).toMatch(/once it approves/);
        expect(r.description.length).toBeLessThanOrEqual(140);
    });

    it("does not carry an OK that later findings superseded", () => {
        const r = evaluateCodexGate({
            headSha: HEAD,
            comments: [okComment(OLD, "2026-09-23T05:00:00Z")],
            reviews: [findingsReview(OLD, "2026-09-23T06:00:00Z")],
            filesSinceLatest: ["docs/notes.md"],
        });
        expect(r.state).toBe("failure");
    });
});

// muxreview no longer re-asks Codex when only doc files it flagged changed
// (agentmuxai/muxreview spec codex-efficiency-2026-09-30 §3), so findings that only
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
        // muxreview re-asks for a non-doc change, so that answer decides the head.
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
        expect(evaluateCodexGate(carried).state).toBe("failure");
    });

    it("treats findings with no known files as before (fail safe)", () => {
        expect(evaluateCodexGate({ headSha: HEAD, reviews: [reviewWithId(11, HEAD)] }).state).toBe("failure");
        const carried = { headSha: HEAD, reviews: [reviewWithId(11, OLD)], filesSinceLatest: ["docs/a.md"] };
        expect(evaluateCodexGate(carried).state).toBe("failure");
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
        expect(r.state).toBe("failure");
    });
});

// While Codex's quota is exhausted muxreview doesn't ask; it keeps one comment
// per PR naming the head it skipped (spec §2).
const SKIP_AUTHOR = "muxreview[bot]";
const skipComment = (sha, { login = SKIP_AUTHOR, created = "2026-09-30T10:00:00Z", updated } = {}) => ({
    user: { login },
    created_at: created,
    ...(updated ? { updated_at: updated } : {}),
    body:
        `Codex not asked about \`${sha.slice(0, 10)}\`: out of review quota until 11:00 UTC. Comment ` +
        "`@muxreview codex re-review` to try sooner.\n" +
        `<!-- reagent:codex-skipped reason=quota head=${sha} -->`,
});

describe("muxreview quota skip marker", () => {
    it("passes the head muxreview skipped", () => {
        const r = evaluateCodexGate({ headSha: HEAD, comments: [skipComment(HEAD)] });
        expect(r.state).toBe("success");
        expect(r.description).toBe(`Codex is out of review quota; ${HEAD.slice(0, 10)} passes without it`);
    });

    it("ignores the same marker from anyone else", () => {
        for (const login of ["someone", "a5af", CODEX_LOGIN, "reagentx-workflow", "muxreview"]) {
            const r = evaluateCodexGate({ headSha: HEAD, comments: [skipComment(HEAD, { login })] });
            expect(r.state).toBe("pending");
        }
    });

    it("counts the marker from the App's own login only", () => {
        const r = evaluateCodexGate({ headSha: HEAD, comments: [skipComment(HEAD, { login: "muxreview[bot]" })] });
        expect(r.state).toBe("success");
    });

    // A name the App used before a rename can be registered by anyone.
    it("does not count the marker from the App's old names", () => {
        for (const login of ["reagentx-workflow[bot]", "reagentx-workflow", "reagent-bot"]) {
            const r = evaluateCodexGate({ headSha: HEAD, comments: [skipComment(HEAD, { login })] });
            expect(r.state).toBe("pending");
        }
    });

    it("follows the comment when muxreview edits it to a newer head", () => {
        const edited = skipComment(HEAD, { created: "2026-09-30T10:00:00Z", updated: "2026-09-30T10:30:00Z" });
        expect(evaluateCodexGate({ headSha: HEAD, comments: [edited] }).state).toBe("success");
        // The edit replaced OLD's marker, so OLD is no longer passed by it.
        expect(evaluateCodexGate({ headSha: OLD, comments: [edited] }).state).toBe("pending");
    });

    // muxreview writes the note before reading Codex's
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
        expect(skipNote({ ...c, body: c.body.replace(HEAD, HEAD.slice(0, 7).toUpperCase()) })?.sha).toBe(HEAD.slice(0, 7));
        expect(skipNote({ ...c, body: c.body.replace(HEAD, "abc12") })).toBeNull();
        expect(skipNote({ ...c, body: c.body.replace("reason=quota", "reason=other") })).toBeNull();
    });
});

// agentmuxai/muxreview: muxreview also skips Codex past a per-PR cap of automatic rounds
// and on docs-only PRs, with the same note and a different reason.
describe("muxreview skip reasons", () => {
    const note = (reason, sha = HEAD) => ({
        user: { login: "muxreview[bot]" },
        created_at: "2026-10-01T12:00:00Z",
        body: `Codex not asked about \`${sha.slice(0, 10)}\`: x.\n\n<!-- reagent:codex-skipped reason=${reason} head=${sha} -->`,
    });

    it("a round-cap skip passes the head and says why", () => {
        const r = evaluateCodexGate({ headSha: HEAD, comments: [note("round-cap")] });
        expect(r.state).toBe("success");
        expect(r.description).toMatch(/round cap/);
    });

    it("a docs-only skip passes the head and says why, once the gate's own check of the files agrees", () => {
        const r = evaluateCodexGate({ headSha: HEAD, comments: [note("docs-only")], noCodePr: true });
        expect(r.state).toBe("success");
        expect(r.description).toMatch(/Docs-only PR/);
    });

    it("a no-code skip passes the head and says why, once the gate's own check of the files agrees", () => {
        const r = evaluateCodexGate({ headSha: HEAD, comments: [note("no-code")], noCodePr: true });
        expect(r.state).toBe("success");
        expect(r.description).toMatch(/No code in this PR/);
    });

    it.each([["no-code"], ["docs-only"]])("a %s skip is refused when the files say there is code, or were not checked", (reason) => {
        for (const noCodePr of [false, null, undefined]) {
            const r = evaluateCodexGate({ headSha: HEAD, comments: [note(reason)], noCodePr, noCodeWhy: "src/main.rs is code" });
            expect(r.state, String(noCodePr)).toBe("pending");
            expect(r.description).toMatch(/no code, but src\/main\.rs is code/);
        }
        expect(evaluateCodexGate({ headSha: HEAD, comments: [note(reason)] }).state).toBe("pending");
    });

    it("a no-code skip for another head, or from anyone but muxreview, passes nothing", () => {
        expect(evaluateCodexGate({ headSha: HEAD, comments: [note("no-code", OLD)], noCodePr: true }).state).toBe("pending");
        const forged = { ...note("no-code"), user: { login: "someone-else" } };
        expect(evaluateCodexGate({ headSha: HEAD, comments: [forged], noCodePr: true }).state).toBe("pending");
        expect(skipClaim({ headSha: HEAD, comments: [forged] })).toBeNull();
    });

    it("the files are read only for a skip that claims there is no code, on this head", () => {
        expect(skipClaim({ headSha: HEAD, comments: [note("no-code")] })).toBe("no-code");
        expect(skipClaim({ headSha: HEAD, comments: [note("docs-only")] })).toBe("docs-only");
        expect(skipClaim({ headSha: HEAD, comments: [note("no-code", OLD)] })).toBeNull();
        expect(skipClaim({ headSha: HEAD, comments: [note("round-cap")] })).toBeNull();
        expect(skipClaim({ headSha: HEAD, comments: [note("quota")] })).toBeNull();
    });

    it("a round-cap or quota skip does not depend on the files", () => {
        expect(evaluateCodexGate({ headSha: HEAD, comments: [note("round-cap")], noCodePr: false }).state).toBe("success");
        expect(evaluateCodexGate({ headSha: HEAD, comments: [note("quota")], noCodePr: false }).state).toBe("success");
    });

    it("Codex's findings on the head outrank a no-code skip", () => {
        const findings = {
            id: 9, user: { login: "chatgpt-codex-connector[bot]" }, state: "COMMENTED",
            submitted_at: "2026-10-01T11:00:00Z", commit_id: HEAD,
            body: `**Reviewed commit:** \`${HEAD.slice(0, 10)}\``,
        };
        const r = evaluateCodexGate({ headSha: HEAD, comments: [note("no-code")], reviews: [findings], noCodePr: true });
        expect(r.state).toBe("failure");
    });

    it("a quota skip keeps its description", () => {
        expect(evaluateCodexGate({ headSha: HEAD, comments: [note("quota")] }).description).toMatch(/out of review quota/);
    });

    it("an unknown reason doesn't count", () => {
        expect(skipNote(note("other"))).toBeNull();
    });

    it("a skip note never outranks Codex's own findings on the head", () => {
        const findings = {
            id: 9, user: { login: "chatgpt-codex-connector[bot]" }, state: "COMMENTED",
            submitted_at: "2026-10-01T11:00:00Z", commit_id: HEAD,
            body: `### 💡 Codex Review\n\nhttps://github.com/agentmuxai/agentmux/blob/${HEAD}/x\n**Reviewed commit:** \`${HEAD.slice(0, 10)}\``,
        };
        const r = evaluateCodexGate({ headSha: HEAD, comments: [note("round-cap")], reviews: [findings] });
        expect(r.state).not.toBe("success");
    });
});

describe("prNoCode", () => {
    const pull = { number: 7, base: { sha: "b".repeat(40) }, head: { sha: HEAD } };
    const MERGE_BASE = "a".repeat(40);
    const manifest = (v) => JSON.stringify({ name: "agentmux", version: v, dependencies: { left: "1.0.0" } }, null, 4);

    function fakeClient({ files, contents = {} }) {
        const calls = [];
        return {
            calls,
            paginate: async (path) => (calls.push(path), files),
            json: async (path) => (calls.push(path), { merge_base_commit: { sha: MERGE_BASE } }),
            request: async (path, init) => {
                calls.push(path);
                expect(init.headers.Accept).toBe("application/vnd.github.raw+json");
                const m = /contents\/(.+)\?ref=(.+)$/.exec(path);
                const key = `${decodeURIComponent(m[1])}@${m[2]}`;
                if (!(key in contents)) throw new GitHubError("GET", path, 404, "");
                return { text: async () => contents[key] };
            },
        };
    }

    it("a release PR reads each version file at the merge base and at the head", async () => {
        const client = fakeClient({
            files: [{ filename: "package.json", status: "modified" }, { filename: "VERSION_HISTORY.md", status: "modified" }],
            contents: { [`package.json@${MERGE_BASE}`]: manifest("0.60.6"), [`package.json@${HEAD}`]: manifest("0.60.7") },
        });
        expect(await prNoCode(client, "o/r", pull)).toMatchObject({ ok: true });
        expect(client.calls).toContain("/repos/o/r/pulls/7/files");
        expect(client.calls).toContain(`/repos/o/r/contents/package.json?ref=${MERGE_BASE}`);
    });

    it("code in the PR is refused, and nothing is read for docs", async () => {
        const client = fakeClient({ files: [{ filename: "docs/a.md", status: "modified" }, { filename: "src/a.rs", status: "modified" }] });
        expect(await prNoCode(client, "o/r", pull)).toEqual({ ok: false, why: "src/a.rs is code" });
        expect(client.calls.some((c) => c.includes("/contents/"))).toBe(false);
    });

    it("a version file that has a dependency change is refused", async () => {
        const dep = JSON.stringify({ name: "agentmux", version: "0.60.7", dependencies: { left: "2.0.0" } }, null, 4);
        const client = fakeClient({
            files: [{ filename: "package.json", status: "modified" }],
            contents: { [`package.json@${MERGE_BASE}`]: manifest("0.60.6"), [`package.json@${HEAD}`]: dep },
        });
        expect(await prNoCode(client, "o/r", pull)).toMatchObject({ ok: false });
    });

    it("a file missing at one of the commits is refused, not an error", async () => {
        const client = fakeClient({ files: [{ filename: "package.json", status: "modified" }], contents: { [`package.json@${HEAD}`]: manifest("0.60.7") } });
        expect(await prNoCode(client, "o/r", pull)).toMatchObject({ ok: false });
    });
});

describe("checkSkipClaim", () => {
    const pull = { number: 7, base: { sha: "b".repeat(40) }, head: { sha: HEAD } };
    const clientOf = (files) => ({ paginate: async () => files });

    it("a docs-only note is confirmed by the older path rule, as before (any file type under docs/)", async () => {
        const files = [{ filename: "docs/specs/SPEC_X.md" }, { filename: "docs/data/sample.json" }, { filename: "README.md" }];
        expect(await checkSkipClaim(clientOf(files), "o/r", pull, "docs-only")).toMatchObject({ ok: true });
    });

    it("a docs-only note on a PR with code, a code rename, or no file list is refused", async () => {
        expect((await checkSkipClaim(clientOf([{ filename: "docs/a.md" }, { filename: "src/a.rs" }]), "o/r", pull, "docs-only")).ok).toBe(false);
        const moved = [{ filename: "docs/a.md", status: "renamed", previous_filename: "src/a.rs" }];
        expect((await checkSkipClaim(clientOf(moved), "o/r", pull, "docs-only")).ok).toBe(false);
        expect((await checkSkipClaim(clientOf([]), "o/r", pull, "docs-only")).ok).toBe(false);
    });
});

// Shaped like the pulls commits API on #4368 (a real Dependabot bump).
const commit = (author, committer = "web-flow") => ({ author: { login: author }, committer: { login: committer } });

describe("dependency bot PRs", () => {
    it("a Dependabot PR of only its own commits counts", () => {
        expect(isDependencyBotPr("dependabot[bot]", [commit("dependabot[bot]")])).toBe(true);
        expect(isDependencyBotPr("renovate[bot]", [commit("renovate[bot]", "renovate[bot]")])).toBe(true);
    });

    it("a commit by anyone else makes it need Codex", () => {
        expect(isDependencyBotPr("dependabot[bot]", [commit("dependabot[bot]"), commit("agent3-workflow[bot]")])).toBe(false);
    });

    it("a commit authored as the bot but committed by someone else does not count", () => {
        expect(isDependencyBotPr("dependabot[bot]", [commit("dependabot[bot]", "someone")])).toBe(false);
    });

    it("another bot's commits on a Dependabot PR do not count", () => {
        expect(isDependencyBotPr("dependabot[bot]", [commit("renovate[bot]")])).toBe(false);
    });

    it("an agent's or a person's PR never counts, even of bot commits", () => {
        expect(isDependencyBotPr("agent3-workflow[bot]", [commit("dependabot[bot]")])).toBe(false);
        expect(isDependencyBotPr("someone", [commit("dependabot[bot]")])).toBe(false);
    });

    it("an empty or unknown commit list does not count", () => {
        expect(isDependencyBotPr("dependabot[bot]", [])).toBe(false);
        expect(isDependencyBotPr("dependabot[bot]", null)).toBe(false);
        expect(isDependencyBotPr(undefined, [commit("dependabot[bot]")])).toBe(false);
    });

    it("passes a dependency bot PR that Codex was never asked about", () => {
        const r = evaluateCodexGate({ headSha: HEAD, dependencyBot: true });
        expect(r.state).toBe("success");
        expect(r.description).toContain("Dependency bot PR");
    });

    it("a verdict Codex did give still counts: findings fail a dependency bot PR", () => {
        const r = evaluateCodexGate({ headSha: HEAD, reviews: [findingsReview(HEAD)], dependencyBot: true });
        expect(r.state).toBe("failure");
    });

    it("without the flag the same PR still waits", () => {
        expect(evaluateCodexGate({ headSha: HEAD }).state).toBe("pending");
    });
});

describe("needsDiffSince", () => {
    it("fetches the diff for an OK or any findings on an earlier commit", () => {
        expect(needsDiffSince({ kind: "ok", sha: OLD.slice(0, 10) }, HEAD)).toBe(true);
        // Code findings too: with only docs since, the gate fails instead of waiting.
        expect(needsDiffSince({ kind: "findings", sha: OLD.slice(0, 10), files: new Set(["src/a.rs"]) }, HEAD)).toBe(true);
    });

    it("skips it for the head itself, a quota answer, or no answer", () => {
        expect(needsDiffSince({ kind: "findings", sha: HEAD.slice(0, 10) }, HEAD)).toBe(false);
        expect(needsDiffSince({ kind: "quota", sha: OLD.slice(0, 10) }, HEAD)).toBe(false);
        expect(needsDiffSince(null, HEAD)).toBe(false);
    });
});

describe("changedFiles", () => {
    const client = (...responses) =>
        createGitHubClient({ token: "t", fetch: async () => responses.shift(), sleep: async () => {}, log: () => {} });
    const json = (body) => new Response(JSON.stringify(body), { status: 200 });

    it("lists the files compare names", async () => {
        const files = await changedFiles(client(json({ files: [{ filename: "a.rs" }] })), "o/r", OLD, HEAD);
        expect(files).toEqual(["a.rs"]);
    });

    it("treats a commit compare can't reach as an unknown diff", async () => {
        for (const code of [404, 422]) {
            expect(await changedFiles(client(new Response("", { status: code })), "o/r", OLD, HEAD)).toBeNull();
        }
    });

    it("fails on an error that outlasts the retries, rather than guessing", async () => {
        const fivehundreds = [1, 2, 3, 4].map(() => new Response("", { status: 500 }));
        await expect(changedFiles(client(...fivehundreds), "o/r", OLD, HEAD)).rejects.toMatchObject({ status: 500 });
    });

    it("recovers from a transient error", async () => {
        const files = await changedFiles(client(new Response("", { status: 502 }), json({ files: [] })), "o/r", OLD, HEAD);
        expect(files).toEqual([]);
    });
});

// That each job's sparse checkout covers the gate's imports is checked for
// every workflow by scripts/workflow-sparse-checkout.test.mjs.
describe("the workflow that runs the gate", () => {
    // WORKFLOW_FILE is the file the sweep dispatches; reading it proves it exists.
    const workflow = readFileSync(new URL(`../.github/workflows/${WORKFLOW_FILE}`, import.meta.url), "utf8");

    it("takes the pr input the sweep sends", () => {
        expect(workflow).toMatch(/workflow_dispatch:\s*\n\s*inputs:\s*\n\s*pr:/);
    });

    // Its own file since 2026-10-19: as a job here, its scheduled runs never
    // started (SPEC_CI_MERGE_QUEUE_AND_FAST_GATES_2026_10_19 §5 Phase 6).
    it("leaves the sweep to codex-review-gate-sweep.yml, which runs it on a schedule", () => {
        expect(workflow).not.toMatch(/^  schedule:/m);
        const sweep = readFileSync(new URL("../.github/workflows/codex-review-gate-sweep.yml", import.meta.url), "utf8");
        expect(sweep).toMatch(/^  schedule:\s*\n\s*- cron:/m);
        expect(sweep).toContain('SWEEP: "1"');
        expect(sweep).toMatch(/actions: write/);
    });

    it("checks out the workflow's own commit on the default branch, and never PR code", () => {
        // A bare github.sha would be the PR's merge commit on pull_request_review.
        expect(workflow).toContain(
            "ref: ${{ github.ref == format('refs/heads/{0}', github.event.repository.default_branch) && github.sha || github.event.repository.default_branch }}",
        );
        expect(workflow).not.toMatch(/ref: \$\{\{ github\.sha \}\}/);
    });

    it("runs on comments from every login the gate reads", () => {
        for (const login of [CODEX_LOGIN, ...SKIP_AUTHORS]) expect(workflow).toContain(`'${login}'`);
        expect(workflow).toContain("'reagent:codex-skipped'");
    });
});

// #5400: Codex accepted the request at 08:56 and never finished; the next request, at 17:24,
// was answered in 3 minutes. The status said nothing about how long it had waited.
describe("a request Codex has left unanswered", () => {
    const ASKED = "2026-10-24T08:56:08Z";
    const at = (minutesAfter) => new Date(Date.parse(ASKED) + minutesAfter * 60_000).toISOString();
    const runningSummary = (t) => ({
        // Codex's "running" summary names no commit: it is not an answer.
        user: { login: CODEX_LOGIN },
        created_at: t,
        body: "<!-- codex-pull-request-review-summary -->\n## Codex Review Summary\n\n| Code Review | Running |",
    });

    it("is the latest request for the head with no verdict after it", () => {
        const comments = [trigger(HEAD, ASKED), runningSummary("2026-10-24T08:56:22Z")];
        expect(unansweredRequest({ headSha: HEAD, comments })).toEqual({ at: ASKED });
    });

    it("is answered by a verdict, a findings review or a usage notice for the head, posted after it", () => {
        for (const extra of [{ comments: [okComment(HEAD, at(3))] }, { reviews: [findingsReview(HEAD, at(3))] }]) {
            const comments = [trigger(HEAD, ASKED), ...(extra.comments ?? [])];
            expect(unansweredRequest({ headSha: HEAD, comments, reviews: extra.reviews ?? [] })).toBeNull();
        }
        // A notice with exactly one request outstanding answers that head.
        expect(unansweredRequest({ headSha: HEAD, comments: [trigger(HEAD, ASKED), quotaComment(at(1))] })).toBeNull();
    });

    it("is not answered by a verdict about another commit, or one posted before the request", () => {
        expect(unansweredRequest({ headSha: HEAD, comments: [trigger(HEAD, ASKED), okComment(OLD, at(3))] })).toEqual({ at: ASKED });
        expect(unansweredRequest({ headSha: HEAD, comments: [okComment(HEAD, at(-5)), trigger(HEAD, ASKED)] })).toEqual({ at: ASKED });
    });

    it("ignores requests for another head, and requests not from muxreview's a5af", () => {
        expect(unansweredRequest({ headSha: HEAD, comments: [trigger(OLD, ASKED)] })).toBeNull();
        expect(unansweredRequest({ headSha: HEAD, comments: [trigger(HEAD, ASKED, "someone-else")] })).toBeNull();
        expect(unansweredRequest({ headSha: HEAD })).toBeNull();
    });

    it("counts from the newest request when it is asked again", () => {
        const comments = [trigger(HEAD, ASKED), okComment(HEAD, at(3)), trigger(HEAD, at(120))];
        // The OK answered the first request, not the second: a later verdict is what answers it.
        expect(unansweredRequest({ headSha: HEAD, comments })).toEqual({ at: at(120) });
    });

    it("makes the pending status say how long, and what to do, once past muxreview's window", () => {
        const comments = [trigger(HEAD, ASKED)];
        const r = evaluateCodexGate({ headSha: HEAD, comments, now: Date.parse(at(8 * 60 + 28)) });
        expect(r.state).toBe("pending");
        expect(r.description).toBe(
            `Codex has not answered the request for ${HEAD.slice(0, 10)} in 8 h 28 min; comment '@muxreview codex re-review'`
        );
        expect(r.description.length).toBeLessThanOrEqual(140);
    });

    it("keeps today's wording while the request is younger than the window, or when there is none", () => {
        const comments = [trigger(HEAD, ASKED)];
        const young = evaluateCodexGate({ headSha: HEAD, comments, now: Date.parse(at(29)) });
        expect(young.description).toMatch(/^Waiting for Codex on/);
        expect(evaluateCodexGate({ headSha: HEAD, now: Date.parse(at(600)) }).description).toMatch(/^Waiting for Codex on/);
        const edge = evaluateCodexGate({ headSha: HEAD, comments, now: Date.parse(at(30)) });
        expect(edge.description).toMatch(/has not answered the request .* in 30 min/);
    });

    it("also shows it where Codex's findings on an earlier head are waiting for the next round", () => {
        const comments = [trigger(HEAD, ASKED)];
        const reviews = [findingsReview(OLD, "2026-10-24T08:50:00Z")];
        const r = evaluateCodexGate({ headSha: HEAD, comments, reviews, filesSinceLatest: ["crates/a.rs"], now: Date.parse(at(95)) });
        expect(r.state).toBe("pending");
        expect(r.description).toMatch(/has not answered the request for .* in 1 h 35 min/);
    });

    it("never turns a pass or a failure pending", () => {
        const comments = [trigger(HEAD, ASKED), okComment(HEAD, at(3))];
        expect(evaluateCodexGate({ headSha: HEAD, comments, now: Date.parse(at(900)) }).state).toBe("success");
        const failing = evaluateCodexGate({ headSha: HEAD, comments: [trigger(HEAD, ASKED)], reviews: [findingsReview(HEAD, at(3))], now: Date.parse(at(900)) });
        expect(failing.state).toBe("failure");
    });

    it("formats ages", () => {
        expect(ageText(0)).toBe("0 min");
        expect(ageText(59 * 60_000 + 59_000)).toBe("59 min");
        expect(ageText(60 * 60_000)).toBe("1 h");
        expect(ageText(125 * 60_000)).toBe("2 h 5 min");
        expect(ageText(-5)).toBe("0 min");
    });

    it("is refreshed by the sweep as it ages, and only then", () => {
        const pull = { number: 5400, draft: false, head: { sha: HEAD } };
        const pending = (t) => ({ context: "Codex review", state: "pending", updated_at: t });
        const comments = [trigger(HEAD, ASKED)];
        const now = Date.parse(at(95));
        // The status was set 40 minutes ago (older than the refresh step): set it again.
        expect(sweepReason({ pull, status: pending(at(55)), comments, now })).toMatch(/refreshing the age/);
        // Set 10 minutes ago: nothing to do yet.
        expect(sweepReason({ pull, status: pending(at(85)), comments, now })).toBeNull();
        // The request is younger than the window: the status has no age to show.
        expect(sweepReason({ pull, status: pending(at(0)), comments, now: Date.parse(at(29)) })).toBeNull();
        // Answered: the sweep's other rules decide, not this one.
        expect(sweepReason({ pull, status: pending(at(55)), comments: [...comments, okComment(HEAD, at(60))], now })).toMatch(/newer than its pending status/);
        expect(STALE_REQUEST_MS).toBe(30 * 60_000);
        expect(AGE_REFRESH_MS).toBeLessThan(STALE_REQUEST_MS);
    });
});

describe("sweepReason", () => {
    const pull = { number: 7, draft: false, head: { sha: HEAD } };
    const pending = (at) => ({ context: "Codex review", state: "pending", updated_at: at });

    it("picks a head with no Codex review status at all", () => {
        expect(sweepReason({ pull, status: null })).toMatch(/no Codex review status/);
    });

    it("picks a pending status older than Codex's quota notice (#4863)", () => {
        const reason = sweepReason({ pull, status: pending("2026-10-14T07:14:39Z"), comments: [quotaComment("2026-10-14T07:15:48Z")] });
        expect(reason).toMatch(/newer than its pending status/);
    });

    it("picks a pending status older than a Codex findings review", () => {
        const reason = sweepReason({ pull, status: pending("2026-09-23T05:00:00Z"), reviews: [findingsReview(HEAD)] });
        expect(reason).toMatch(/Codex review is newer/);
    });

    it("picks a pending status older than a muxreview skip note's latest edit", () => {
        const note = skipComment(HEAD, { created: "2026-09-30T10:00:00Z", updated: "2026-09-30T12:00:00Z" });
        expect(sweepReason({ pull, status: pending("2026-09-30T11:00:00Z"), comments: [note] })).not.toBeNull();
    });

    it("leaves a pending status alone when every gate input is older", () => {
        const comments = [quotaComment("2026-10-14T07:15:48Z")];
        expect(sweepReason({ pull, status: pending("2026-10-14T07:15:59Z"), comments })).toBeNull();
    });

    it("leaves a PR alone that is only waiting, whatever else was said", () => {
        const comments = [okComment(HEAD, "2026-10-14T08:00:00Z", "someone"), skipComment(HEAD, { login: "someone", created: "2026-10-14T08:00:00Z" })];
        expect(sweepReason({ pull, status: pending("2026-10-14T07:00:00Z"), comments })).toBeNull();
    });

    it("never touches a decided status or a draft", () => {
        const comments = [quotaComment("2026-10-14T09:00:00Z")];
        for (const state of ["success", "failure", "error"]) {
            expect(sweepReason({ pull, status: { state, updated_at: "2026-10-14T07:00:00Z" }, comments })).toBeNull();
        }
        expect(sweepReason({ pull: { ...pull, draft: true }, status: null })).toBeNull();
    });
});

describe("sweep", () => {
    const pr = (number, sha, extra = {}) => ({ number, draft: false, head: { sha }, ...extra });
    const SHA = { a: "a".repeat(40), b: "b".repeat(40), c: "c".repeat(40), d: "d".repeat(40) };

    /** A fake client over canned responses, recording dispatches. */
    function fakeClient({ pulls, statuses, comments = {}, failStatusFor = null }) {
        const dispatched = [];
        const reads = [];
        return {
            dispatched,
            reads,
            paginate: async (path) => {
                reads.push(path);
                if (path.endsWith("/pulls?state=open")) return pulls;
                const n = Number(/\/(\d+)\/(comments|reviews)$/.exec(path)?.[1]);
                return path.endsWith("/comments") ? (comments[n] ?? []) : [];
            },
            json: async (path) => {
                const sha = /commits\/([0-9a-f]+)\/status$/.exec(path)[1];
                if (sha === failStatusFor) throw new Error("boom");
                return { statuses: statuses[sha] ? [statuses[sha]] : [] };
            },
            request: async (path, init) => {
                dispatched.push({ path, body: JSON.parse(init.body) });
            },
        };
    }

    it("dispatches the gate only for PRs that missed an input", async () => {
        const client = fakeClient({
            pulls: [pr(1, SHA.a), pr(2, SHA.b), pr(3, SHA.c), pr(4, SHA.d, { draft: true })],
            statuses: {
                // 1: none at all. 2: pending, older than Codex's quota notice. 3: decided.
                [SHA.b]: { context: "Codex review", state: "pending", updated_at: "2026-10-14T07:14:39Z" },
                [SHA.c]: { context: "Codex review", state: "success", updated_at: "2026-10-14T07:00:00Z" },
            },
            comments: { 2: [quotaComment("2026-10-14T07:15:48Z")] },
        });
        const logs = [];
        expect(await sweep(client, "o/r", { ref: "main", log: (l) => logs.push(l) })).toEqual([1, 2]);
        expect(client.dispatched).toEqual([
            { path: `/repos/o/r/actions/workflows/${WORKFLOW_FILE}/dispatches`, body: { ref: "main", inputs: { pr: "1" } } },
            { path: `/repos/o/r/actions/workflows/${WORKFLOW_FILE}/dispatches`, body: { ref: "main", inputs: { pr: "2" } } },
        ]);
        expect(logs.at(-1)).toBe("sweep: 4 open PR(s), 2 dispatched, 0 failed");
        // Comments are read only where a pending status could be stale.
        expect(client.reads.filter((p) => p.includes("/issues/"))).toEqual(["/repos/o/r/issues/2/comments"]);
    });

    it("dispatches nothing on a dry run", async () => {
        const client = fakeClient({ pulls: [pr(1, SHA.a)], statuses: {} });
        expect(await sweep(client, "o/r", { ref: "main", dryRun: true, log: () => {} })).toEqual([1]);
        expect(client.dispatched).toEqual([]);
    });

    it("carries on past one PR's error, then fails the run", async () => {
        const client = fakeClient({ pulls: [pr(1, SHA.a), pr(2, SHA.b)], statuses: {}, failStatusFor: SHA.a });
        await expect(sweep(client, "o/r", { ref: "main", log: () => {} })).rejects.toThrow("sweep failed for #1");
        expect(client.dispatched.map((d) => d.body.inputs.pr)).toEqual(["2"]);
    });
});
