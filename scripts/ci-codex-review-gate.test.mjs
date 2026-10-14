// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for ci-codex-review-gate.mjs. The gate's one job is to never go
// green for a commit Codex has not OK'd, so most cases here are ways it
// could wrongly succeed.

import { readFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { createGitHubClient } from "./lib/github-rest.mjs";
import {
    changedFiles,
    CODEX_LOGIN,
    evaluateCodexGate,
    isDependencyBotPr,
    isDocsOnlyPath,
    latestCodexOutput,
    needsDiffSince,
    reviewedCommit,
    SKIP_AUTHORS,
    skipNote,
    sweep,
    sweepReason,
    TRIGGER_AUTHOR,
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
        "`@reagentx-workflow codex re-review` to try sooner.\n" +
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

    // The App was renamed reagentx-workflow -> muxreview on 2026-10-07.
    it("counts the marker from the renamed App and from its old login", () => {
        for (const login of ["muxreview[bot]", "reagentx-workflow[bot]"]) {
            const r = evaluateCodexGate({ headSha: HEAD, comments: [skipComment(HEAD, { login })] });
            expect(r.state).toBe("success");
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
        user: { login: "reagentx-workflow[bot]" },
        created_at: "2026-10-01T12:00:00Z",
        body: `Codex not asked about \`${sha.slice(0, 10)}\`: x.\n\n<!-- reagent:codex-skipped reason=${reason} head=${sha} -->`,
    });

    it("a round-cap skip passes the head and says why", () => {
        const r = evaluateCodexGate({ headSha: HEAD, comments: [note("round-cap")] });
        expect(r.state).toBe("success");
        expect(r.description).toMatch(/round cap/);
    });

    it("a docs-only skip passes the head and says why", () => {
        const r = evaluateCodexGate({ headSha: HEAD, comments: [note("docs-only")] });
        expect(r.state).toBe("success");
        expect(r.description).toMatch(/Docs-only PR/);
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

describe("the workflow that runs the gate", () => {
    const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
    const workflow = readFileSync(join(repoRoot, ".github/workflows/codex-review-gate.yml"), "utf8");

    /** Repo-relative paths of the gate and every module it imports, transitively. */
    function gateModules(entry = "scripts/ci-codex-review-gate.mjs", seen = new Set()) {
        if (seen.has(entry)) return seen;
        seen.add(entry);
        const source = readFileSync(join(repoRoot, entry), "utf8");
        for (const [, spec] of source.matchAll(/^\s*(?:import|export)\b[^;]*?\bfrom\s+["'](\.{1,2}\/[^"']+)["']/gm)) {
            gateModules(relative(repoRoot, join(repoRoot, dirname(entry), spec)).replace(/\\/g, "/"), seen);
        }
        return seen;
    }

    /** The paths under each `sparse-checkout:` key, one list per checkout. */
    function sparseCheckouts() {
        const lines = workflow.split("\n");
        const lists = [];
        lines.forEach((line, i) => {
            const m = /^(\s*)sparse-checkout:\s*(.*)$/.exec(line);
            if (!m) return;
            if (m[2].trim() !== "|") return lists.push([m[2].trim()]);
            const paths = [];
            for (const next of lines.slice(i + 1)) {
                if (next.trim() === "" || next.search(/\S/) <= m[1].length) break;
                paths.push(next.trim());
            }
            lists.push(paths);
        });
        return lists;
    }

    it("checks out every module the gate imports, in every job that runs it", () => {
        const modules = [...gateModules()];
        expect(modules).toContain("scripts/lib/github-rest.mjs");
        const lists = sparseCheckouts();
        expect(lists).toHaveLength(workflow.match(/run: node scripts\/ci-codex-review-gate\.mjs/g).length);
        for (const paths of lists) expect(paths).toEqual(expect.arrayContaining(modules));
    });

    it("is the workflow the sweep dispatches, with the pr input it sends", () => {
        expect(readFileSync(join(repoRoot, ".github/workflows", WORKFLOW_FILE), "utf8")).toBe(workflow);
        expect(workflow).toMatch(/workflow_dispatch:\s*\n\s*inputs:\s*\n\s*pr:/);
        expect(workflow).toMatch(/sweep:\s*\n\s*if: github\.event_name == 'schedule'/);
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
