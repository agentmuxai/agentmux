// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Sets the `Codex review` commit status on a PR's head: success only once
// Codex has said "Didn't find any major issues" about that exact commit, or
// answered the request for it with its out-of-quota notice. A dependency
// bot's own PR passes without Codex (see DEPENDENCY_BOTS).
// Run by .github/workflows/codex-review-gate.yml.
//
// Why a status keyed to the head commit: #3513 merged at 18:07 while Codex
// was still finding real bugs in it; its OK came at 20:02, and three fixes
// were orphaned (#3538). A push after an OK needs a fresh OK, so the OK is
// matched on the "Reviewed commit" Codex prints, never on "Codex said OK
// somewhere in this PR".
//
// What Codex posts (observed on #3513/#3523/#3538/#3562):
//   - no findings: an ISSUE COMMENT "Codex Review: Didn't find any major
//     issues. ..." with "**Reviewed commit:** `<10-char sha>`"
//   - findings:    a PR REVIEW (state COMMENTED) with the same
//     "Reviewed commit" line and inline comments.
//   - out of quota: an ISSUE COMMENT "You have reached your Codex usage
//     limits for code reviews. ..." naming no commit. When it answers the
//     only outstanding muxreview trigger ("@codex review" by a5af with
//     `<!-- reagent:codex-trigger head=<sha> -->`), it counts for that
//     head and passes it: Codex being unavailable must not block merges
//     (#3562 sat pending). See quotaAttribution for overlapping requests.
//     A later real verdict on the head still wins.
//
// What muxreview posts:
//   - quota skip: while Codex's quota is exhausted muxreview stops asking and
//     instead keeps one ISSUE COMMENT per PR, edited in place for each head
//     it skips, carrying `<!-- reagent:codex-skipped reason=quota
//     head=<sha> -->`. From muxreview's App only (SKIP_AUTHORS; anyone can
//     type the marker), it counts as a quota answer for that head, but only when
//     Codex itself has said nothing about the head: muxreview may write it
//     before reading Codex's answers. It is never carried to a later head.
//
// Codex only reviews when asked by a5af, not a bot. muxreview decides when
// to ask: after it approves a head,
// then again after it approves a push that changes code (any non-doc file,
// flagged by Codex or not), or when a write-access commenter says
// "@muxreview codex re-review". After an OK or findings it does NOT re-ask
// for a docs-only diff. So this gate carries an OK across exactly that diff;
// findings whose inline comments (matched to the review by
// pull_request_review_id) are all on docs pass on the head and carry to a
// later head like an OK, only while they are Codex's latest word and only
// docs changed since. Other findings on an earlier head wait for the re-ask
// when code changed since, and fail when only docs did: nothing will ask
// Codex again, so the PR needs the fix. This gate only reads.
//
// If the status looks stuck (Codex answered but the status didn't move),
// re-evaluate the PR without asking Codex anything:
//   gh workflow run codex-review-gate.yml -f pr=<number>
// GitHub calls go through scripts/lib/github-rest.mjs, which retries 5xx and
// rate limits; the workflow's sparse checkout must list every module this one
// imports (a test checks it).

import { createGitHubClient, GitHubError } from "./lib/github-rest.mjs";

export const CODEX_LOGIN = "chatgpt-codex-connector[bot]";
export const STATUS_CONTEXT = "Codex review";

// Only muxreview's trigger names the head it asked about; muxreview reads the
// same marker from the same author.
export const TRIGGER_AUTHOR = "a5af";

// muxreview's GitHub App, the only author whose quota-skip marker counts.
// The App was renamed reagentx-workflow -> muxreview on 2026-10-07; markers
// posted before the rename keep the old login. The `reagent:` prefix of the
// markers is muxreview's wire format and stays.
export const SKIP_AUTHORS = new Set(["muxreview[bot]", "reagentx-workflow[bot]"]);

const isCodex = (x) => x?.user?.login === CODEX_LOGIN;
// GitHub's ISO-8601 timestamps sort as strings.
const byAt = (a, b) => String(a.at).localeCompare(String(b.at));
/** Whether `sha` (full or abbreviated) is the commit Codex's `prefix` names. */
const namesCommit = (sha, prefix) => String(sha).toLowerCase().startsWith(prefix);

// Dependency bots. muxreview doesn't review their PRs, so it never asks Codex,
// and the gate would wait forever. A pure version bump is a manifest and
// lockfile diff Codex has little to say about, so such a PR passes without
// Codex; the required approving review and CI still apply. Only while every
// commit is the bot's own: once anyone else pushes to the branch (a code fix
// for a breaking bump), it needs Codex.
export const DEPENDENCY_BOTS = new Set(["dependabot[bot]", "renovate[bot]"]);
// GitHub's own committer for commits an App makes through the API.
const GITHUB_COMMITTER = "web-flow";

/**
 * Whether a PR is a dependency bot's own: opened by one, and every commit
 * (from the pulls commits API) authored by that bot and committed by it or by
 * GitHub for it. An empty or unknown commit list is not.
 */
export function isDependencyBotPr(prAuthor, commits) {
    const bot = String(prAuthor ?? "").toLowerCase();
    if (!DEPENDENCY_BOTS.has(bot) || !Array.isArray(commits) || commits.length === 0) return false;
    return commits.every((c) => {
        const author = String(c?.author?.login ?? "").toLowerCase();
        const committer = String(c?.committer?.login ?? "").toLowerCase();
        return author === bot && (committer === bot || committer === GITHUB_COMMITTER);
    });
}

const REVIEWED_COMMIT = /Reviewed commit:\**\s*`([0-9a-f]{7,40})`/i;
// Straight or curly apostrophe.
const NO_MAJOR_ISSUES = /Didn.t find any major issues/i;
const USAGE_LIMIT = /reached your Codex usage limits/i;
const TRIGGER_HEAD = /reagent:codex-trigger\s+head=([0-9a-f]{7,40})/i;
// reason: quota (Codex out of quota), round-cap (muxreview stopped asking after
// its per-PR cap of automatic rounds) or docs-only (a PR of only docs).
const SKIPPED_HEAD = /<!--\s*reagent:codex-skipped\s+reason=(quota|round-cap|docs-only)\s+head=([0-9a-f]{7,40})\s*-->/i;

// muxreview keeps a mirror of this rule; keep the two in step.
// Narrower than ci-classify-changes.mjs's rule on purpose: CLAUDE.md, AGENTS.md
// and prompt files are markdown that agents act on, so changing them is a
// behavior change Codex should see.
const NEVER_DOCS_PREFIXES = [".github/", "scripts/", "tools/", "prompts/"];
// Both changeset spellings, matching muxreview: plural here, singular upstream.
const DOCS_PREFIXES = ["docs/", ".changesets/", ".changeset/"];
const DOCS_NAMES = /^(README|CHANGELOG)(\.[a-z]+)?$|^(LICENSE|NOTICE)$/i;

export function isDocsOnlyPath(rawPath) {
    let p = String(rawPath ?? "").trim().replace(/\\/g, "/");
    if (p.startsWith("./")) p = p.slice(2);
    if (p === "" || NEVER_DOCS_PREFIXES.some((x) => p.startsWith(x))) return false;
    if (DOCS_PREFIXES.some((x) => p.startsWith(x))) return true;
    return DOCS_NAMES.test(p.split("/").at(-1));
}

export function reviewedCommit(body) {
    const m = REVIEWED_COMMIT.exec(body ?? "");
    return m ? m[1].toLowerCase() : null;
}

/**
 * The head each quota notice answered (Map: notice -> sha), from the
 * timeline of muxreview triggers and Codex answers. The notice carries no
 * request id, so the requests still outstanding when it arrives decide:
 *   - an answer naming a commit closes only the requests for that commit
 *     (Codex P1 on #3589: a blanket "since Codex last spoke" cutoff also
 *     dropped a newer request, leaving its head pending forever);
 *   - a notice with exactly one request outstanding answers that head;
 *   - with several outstanding it is ambiguous (Codex P1 on #3589: crediting
 *     the latest would pass an unreviewed head). It credits no head, closes
 *     them all, and the next request is then answered alone.
 */
function quotaAttribution({ comments, reviews }) {
    const events = [];
    for (const c of comments) {
        const at = String(c.created_at);
        if (c.user?.login === TRIGGER_AUTHOR) {
            const m = TRIGGER_HEAD.exec(c.body ?? "");
            if (m) events.push({ at, order: 0, request: m[1].toLowerCase() });
        } else if (isCodex(c)) {
            const names = reviewedCommit(c.body);
            if (USAGE_LIMIT.test(c.body ?? "")) events.push({ at, order: 1, quota: c });
            else if (names) events.push({ at, order: 1, names });
        }
    }
    for (const r of reviews) {
        const names = isCodex(r) ? reviewedCommit(r.body) : null;
        if (names) events.push({ at: String(r.submitted_at), order: 1, names });
    }
    // Requests sort before answers posted in the same second.
    events.sort((a, b) => byAt(a, b) || a.order - b.order);

    const attributed = new Map();
    let outstanding = [];
    for (const e of events) {
        if (e.request) outstanding.push(e.request);
        else if (e.names) outstanding = outstanding.filter((head) => !namesCommit(head, e.names));
        else if (e.quota) {
            if (outstanding.length === 1) attributed.set(e.quota, outstanding[0]);
            outstanding = [];
        }
    }
    return attributed;
}

/** A muxreview skip comment's { sha, reason }, or null. */
export function skipNote(comment) {
    if (!SKIP_AUTHORS.has(comment?.user?.login)) return null;
    const m = SKIPPED_HEAD.exec(comment.body ?? "");
    return m ? { reason: m[1].toLowerCase(), sha: m[2].toLowerCase() } : null;
}

/** Review id -> Set of the paths its inline comments are on. */
function filesByReview(reviewComments) {
    const byReview = new Map();
    for (const c of reviewComments) {
        if (c.pull_request_review_id == null || !c.path) continue;
        if (!byReview.has(c.pull_request_review_id)) byReview.set(c.pull_request_review_id, new Set());
        byReview.get(c.pull_request_review_id).add(c.path);
    }
    return byReview;
}

/** Findings whose flagged files are known and all docs. */
function isDocsOnlyFindings(o) {
    return o?.kind === "findings" && o.files.size > 0 && [...o.files].every(isDocsOnlyPath);
}

/**
 * muxreview's quota skips, as `quota` outputs, oldest first. muxreview edits one
 * comment in place per outage, so its last edit dates the skip of the head
 * it now names. Not Codex verdicts: see evaluateCodexGate for precedence.
 */
function muxreviewSkips(comments = []) {
    return comments
        .flatMap((c) => {
            const note = skipNote(c);
            return note ? [{ kind: "quota", ...note, at: c.updated_at ?? c.created_at }] : [];
        })
        .sort(byAt);
}

/**
 * Every Codex verdict that names a commit, oldest first. Findings carry
 * `files`, the paths of their inline comments (empty when `reviewComments`
 * has none for them).
 */
function codexOutputs({ comments = [], reviews = [], reviewComments = [] }) {
    const quotaHeads = quotaAttribution({ comments, reviews });
    const flagged = filesByReview(reviewComments);
    return [
        ...comments
            .filter(isCodex)
            .map((c) =>
                USAGE_LIMIT.test(c.body ?? "")
                    ? { kind: "quota", at: c.created_at, sha: quotaHeads.get(c) ?? null }
                    : {
                          kind: NO_MAJOR_ISSUES.test(c.body ?? "") ? "ok" : "other",
                          at: c.created_at,
                          sha: reviewedCommit(c.body),
                      },
            ),
        // A dismissed findings review no longer counts against a commit. It
        // does not count for it either: only a Codex OK passes.
        ...reviews
            .filter((r) => isCodex(r) && r.state !== "DISMISSED")
            .map((r) => ({
                kind: "findings",
                at: r.submitted_at,
                sha: reviewedCommit(r.body),
                files: flagged.get(r.id) ?? new Set(),
            })),
    ]
        .filter((o) => o.sha !== null)
        .sort(byAt);
}

/** Codex's most recent verdict on any commit of the PR, or null. */
export function latestCodexOutput({ comments = [], reviews = [], reviewComments = [] }) {
    return codexOutputs({ comments, reviews, reviewComments }).at(-1) ?? null;
}

/**
 * Whether `evaluateCodexGate` needs the files changed since Codex's latest
 * verdict: an OK or findings about an earlier commit. An OK or docs-only
 * findings carry across a docs-only diff, and other findings with only docs
 * since fail; any other answer decides without it.
 */
export function needsDiffSince(latest, headSha) {
    return (latest?.kind === "ok" || latest?.kind === "findings") && !namesCommit(headSha, latest.sha);
}

// How a quota-kind answer on the head passes, by the skip note's reason.
// Codex's own out-of-quota notice carries no reason and reads as `quota`.
const PASSES_WITHOUT_CODEX = {
    quota: (short) => `Codex is out of review quota; ${short} passes without it`,
    "round-cap": (short) =>
        `muxreview stopped asking Codex after this PR's round cap; ${short} passes without it ('@muxreview codex re-review' asks anyway)`,
    "docs-only": (short) => `Docs-only PR: Codex isn't asked; ${short} passes without it`,
};

/**
 * Decide the status for `headSha` from the PR's issue comments, reviews and
 * review comments (GitHub REST shapes). The latest Codex output naming this
 * head wins, so a spontaneous findings review after an OK takes the OK back.
 * A muxreview quota skip counts only when Codex has said nothing about the
 * head: muxreview can post or edit it before reading Codex's answers, so its
 * date can trail a real verdict it must not override.
 *
 * With nothing on the head, Codex's latest verdict (never a skip) carries
 * over when it is an OK or findings only on docs, and `filesSinceLatest`
 * (the diff from its commit to the head, null if unknown) is docs-only.
 */
export function evaluateCodexGate({
    headSha,
    comments = [],
    reviews = [],
    reviewComments = [],
    filesSinceLatest = null,
    dependencyBot = false,
}) {
    const head = headSha.toLowerCase();
    const short = head.slice(0, 10);
    const all = codexOutputs({ comments, reviews, reviewComments });
    const namesHead = (o) => namesCommit(head, o.sha);
    const onHead = all.filter(namesHead).at(-1) ?? muxreviewSkips(comments).filter(namesHead).at(-1);

    if (onHead?.kind === "ok") {
        return { state: "success", description: `Codex found no major issues in ${short}` };
    }
    if (onHead?.kind === "quota") {
        return { state: "success", description: PASSES_WITHOUT_CODEX[onHead.reason ?? "quota"](short) };
    }
    if (isDocsOnlyFindings(onHead)) {
        return { state: "success", description: `Codex only flagged docs in ${short}; see its comments` };
    }
    if (onHead?.kind === "findings") {
        return {
            state: "failure",
            description: `Codex left findings on ${short}; push a fix and muxreview asks again once it approves`,
        };
    }
    if (onHead) {
        return { state: "pending", description: `Codex commented on ${short} without an OK` };
    }

    const latest = all.at(-1);
    // muxreview re-asks for any non-doc change after either verdict, so an OK
    // or docs-only findings carry only across a docs-only diff.
    const onlyDocsSince = Array.isArray(filesSinceLatest) && filesSinceLatest.every(isDocsOnlyPath);
    if (latest?.kind === "ok" && onlyDocsSince) {
        return { state: "success", description: `Codex OK on ${latest.sha}; only docs changed since` };
    }
    if (isDocsOnlyFindings(latest) && onlyDocsSince) {
        return { state: "success", description: `Codex only flagged docs in ${latest.sha}; see its comments` };
    }
    // Only where it would otherwise wait: a verdict Codex did give still counts.
    if (dependencyBot) {
        return { state: "success", description: `Dependency bot PR: Codex isn't asked; ${short} passes without it` };
    }
    // Findings on an earlier head: a code change since is asked about once
    // muxreview approves it; with only docs since, nothing will ask again.
    if (latest?.kind === "findings") {
        if (onlyDocsSince) {
            return {
                state: "failure",
                description: `Codex's findings on ${latest.sha} are unanswered: only docs changed since. Push the fix, or '@muxreview codex re-review'`,
            };
        }
        return {
            state: "pending",
            description: `Codex's findings on ${latest.sha}: muxreview asks about ${short} once it approves, or comment '@muxreview codex re-review'`,
        };
    }
    return {
        state: "pending",
        description: `Waiting for Codex on ${short}: muxreview asks after approving, or comment '@muxreview codex re-review'`,
    };
}

/**
 * An issue comment the gate reacts to: anything Codex says, or a muxreview
 * skip note. The workflow's `if:` filter encodes the same rule (a test pins
 * the two together).
 */
export function isGateInput(comment) {
    return isCodex(comment) || skipNote(comment) !== null;
}

/**
 * Why the sweep should re-evaluate a PR, or null when it shouldn't. `status`
 * is the head's latest `Codex review` status (null when it has none). Only a
 * missing status, or a pending one some gate input is newer than, means an
 * evaluation was lost; a PR still waiting on Codex or on muxreview's approval
 * has no such input, so it is left alone.
 */
export function sweepReason({ pull, status, comments = [], reviews = [] }) {
    if (pull?.draft) return null;
    if (!status) return "its head has no Codex review status";
    if (status.state !== "pending") return null;
    const since = Date.parse(status.updated_at);
    const newer = (at) => Date.parse(at) > since;
    if (comments.some((c) => isGateInput(c) && newer(c.updated_at ?? c.created_at))) {
        return "a Codex answer or muxreview note is newer than its pending status";
    }
    if (reviews.some((r) => isCodex(r) && newer(r.submitted_at))) {
        return "a Codex review is newer than its pending status";
    }
    return null;
}

// The compare API lists at most 300 files; a list that long may be cut short.
const COMPARE_FILE_CAP = 300;
// What compare answers when the reviewed commit can't be compared with the
// head (a force-push orphaned it). Anything else, after the client's retries,
// fails the run rather than posting a wrong pending.
const COMPARE_UNKNOWN_STATUSES = new Set([404, 422]);

/** Files changed base...head, or null when that can't be known completely. */
export async function changedFiles(client, repo, base, head) {
    try {
        const cmp = await client.json(`/repos/${repo}/compare/${base}...${head}`);
        const files = (cmp.files ?? []).map((f) => f.filename);
        return files.length >= COMPARE_FILE_CAP ? null : files;
    } catch (err) {
        if (!(err instanceof GitHubError) || !COMPARE_UNKNOWN_STATUSES.has(err.status)) throw err;
        console.log(`compare ${base}...${head.slice(0, 10)}: HTTP ${err.status}, treating the diff as unknown`);
        return null;
    }
}

export const WORKFLOW_FILE = "codex-review-gate.yml";

/** The head's latest `Codex review` status, or null. */
async function gateStatus(client, repo, sha) {
    const combined = await client.json(`/repos/${repo}/commits/${sha}/status`);
    return (combined.statuses ?? []).find((s) => s.context === STATUS_CONTEXT) ?? null;
}

/**
 * Dispatches the gate's workflow for every open PR `sweepReason` picks.
 * One PR's error doesn't stop the others; any error fails the run at the end.
 * Returns the PR numbers dispatched.
 */
export async function sweep(client, repo, { ref, dryRun = false, log = console.log } = {}) {
    const pulls = await client.paginate(`/repos/${repo}/pulls?state=open`);
    const dispatched = [];
    const failed = [];
    for (const pull of pulls) {
        try {
            if (pull.draft) continue;
            const status = await gateStatus(client, repo, pull.head.sha);
            // Only a pending status can be older than an input it missed.
            const [comments, reviews] =
                status?.state === "pending"
                    ? await Promise.all([
                          client.paginate(`/repos/${repo}/issues/${pull.number}/comments`),
                          client.paginate(`/repos/${repo}/pulls/${pull.number}/reviews`),
                      ])
                    : [[], []];
            const reason = sweepReason({ pull, status, comments, reviews });
            if (!reason) continue;
            log(`#${pull.number} ${pull.head.sha.slice(0, 10)}: ${reason}; ${dryRun ? "would dispatch" : "dispatching"}`);
            if (!dryRun) {
                await client.request(`/repos/${repo}/actions/workflows/${WORKFLOW_FILE}/dispatches`, {
                    method: "POST",
                    headers: { "Content-Type": "application/json" },
                    body: JSON.stringify({ ref, inputs: { pr: String(pull.number) } }),
                });
            }
            dispatched.push(pull.number);
        } catch (err) {
            log(`#${pull.number}: ${err.message}`);
            failed.push(pull.number);
        }
    }
    log(`sweep: ${pulls.length} open PR(s), ${dispatched.length} ${dryRun ? "to dispatch" : "dispatched"}, ${failed.length} failed`);
    if (failed.length) throw new Error(`sweep failed for #${failed.join(", #")}`);
    return dispatched;
}

async function main() {
    const { GITHUB_TOKEN: token, GITHUB_REPOSITORY: repo, PR_NUMBER: pr, SWEEP, DISPATCH_REF, DRY_RUN } = process.env;
    if (!token || !repo || !(pr || SWEEP)) {
        throw new Error("GITHUB_TOKEN, GITHUB_REPOSITORY and PR_NUMBER (or SWEEP) are required");
    }
    const client = createGitHubClient({ token });
    if (SWEEP) {
        await sweep(client, repo, { ref: DISPATCH_REF || "main", dryRun: Boolean(DRY_RUN) });
        return;
    }
    const pull = await client.json(`/repos/${repo}/pulls/${pr}`);
    const headSha = pull.head.sha;
    const [comments, reviews, reviewComments] = await Promise.all([
        client.paginate(`/repos/${repo}/issues/${pr}/comments`),
        client.paginate(`/repos/${repo}/pulls/${pr}/reviews`),
        // Inline comments: the files each findings review flagged.
        client.paginate(`/repos/${repo}/pulls/${pr}/comments`),
    ]);
    const latest = latestCodexOutput({ comments, reviews, reviewComments });
    const filesSinceLatest = needsDiffSince(latest, headSha) ? await changedFiles(client, repo, latest.sha, headSha) : null;
    // The pulls commits API lists at most 250; a longer PR isn't a bot bump.
    const dependencyBot =
        DEPENDENCY_BOTS.has(String(pull.user?.login ?? "").toLowerCase()) &&
        pull.commits <= 250 &&
        isDependencyBotPr(pull.user.login, await client.paginate(`/repos/${repo}/pulls/${pr}/commits`));
    const result = evaluateCodexGate({ headSha, comments, reviews, reviewComments, filesSinceLatest, dependencyBot });
    console.log(`#${pr} ${headSha.slice(0, 10)}: ${result.state} — ${result.description}`);
    if (DRY_RUN) return;
    await client.request(`/repos/${repo}/statuses/${headSha}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            state: result.state,
            context: STATUS_CONTEXT,
            description: result.description.slice(0, 140),
            target_url: pull.html_url,
        }),
    });
}

if (import.meta.url === `file://${process.argv[1]}` || process.argv[1]?.endsWith("ci-codex-review-gate.mjs")) {
    main().catch((err) => {
        console.error(err.message);
        process.exit(1);
    });
}
