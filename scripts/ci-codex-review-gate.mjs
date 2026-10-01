// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Sets the `Codex review` commit status on a PR's head: success only once
// Codex has said "Didn't find any major issues" about that exact commit, or
// answered the request for it with its out-of-quota notice.
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
//     only outstanding ReAgent trigger ("@codex review" by a5af with
//     `<!-- reagent:codex-trigger head=<sha> -->`), it counts for that
//     head and passes it: Codex being unavailable must not block merges
//     (#3562 sat pending). See quotaAttribution for overlapping requests.
//     A later real verdict on the head still wins.
//
// What ReAgent posts (a5af/reagent spec codex-efficiency-2026-09-30):
//   - quota skip: while Codex's quota is exhausted ReAgent stops asking and
//     instead keeps one ISSUE COMMENT per PR, edited in place for each head
//     it skips, carrying `<!-- reagent:codex-skipped reason=quota
//     head=<sha> -->`. From reagentx-workflow[bot] only (anyone can type the
//     marker), it counts as a quota answer for that head, but only when
//     Codex itself has said nothing about the head: ReAgent may write it
//     before reading Codex's answers. It is never carried to a later head.
//
// Codex only reviews when asked by a5af, not a bot. ReAgent decides when
// to ask (a5af/reagent lambdas/codex_policy.py): after it approves a head,
// then again only when a push touches a file Codex flagged, or when a
// write-access commenter says "@reagentx-workflow codex re-review". After an
// OK it does NOT re-ask for a docs-only diff, so this gate carries an OK
// across exactly that diff. After findings it does NOT re-ask when only doc
// files Codex flagged changed, so findings whose inline comments (matched to
// the review by pull_request_review_id) are all on docs pass on the head, and
// carry to a later head like an OK: only while they are Codex's latest word
// and only docs changed since (a code change makes ReAgent re-ask). Findings
// on any non-doc file, or on files unknown, fail or wait as before. This
// gate only reads.

export const CODEX_LOGIN = "chatgpt-codex-connector[bot]";
export const STATUS_CONTEXT = "Codex review";

// Only ReAgent's trigger names the head it asked about; reagent's
// codex_policy.py reads the same marker from the same author.
export const TRIGGER_AUTHOR = "a5af";

// ReAgent's GitHub App, the only author whose quota-skip marker counts.
export const SKIP_AUTHOR = "reagentx-workflow[bot]";

const REVIEWED_COMMIT = /Reviewed commit:\**\s*`([0-9a-f]{7,40})`/i;
// Straight or curly apostrophe.
const NO_MAJOR_ISSUES = /Didn.t find any major issues/i;
const USAGE_LIMIT = /reached your Codex usage limits/i;
const TRIGGER_HEAD = /reagent:codex-trigger\s+head=([0-9a-f]{7,40})/i;
// reason: quota (Codex out of quota), round-cap (ReAgent stopped asking after
// its per-PR cap of automatic rounds) or docs-only (a PR of only docs).
const SKIPPED_HEAD = /<!--\s*reagent:codex-skipped\s+reason=(quota|round-cap|docs-only)\s+head=([0-9a-f]{7,40})\s*-->/i;

// Mirrors is_docs_only_path in reagent's codex_policy.py; keep them in step.
// Narrower than ci-classify-changes.mjs's rule on purpose: CLAUDE.md, AGENTS.md
// and prompt files are markdown that agents act on, so changing them is a
// behavior change Codex should see.
const NEVER_DOCS_PREFIXES = [".github/", "scripts/", "tools/", "prompts/"];
// Both changeset spellings, matching reagent: plural here, singular upstream.
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
 * timeline of ReAgent triggers and Codex answers. The notice carries no
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
        } else if (c.user?.login === CODEX_LOGIN) {
            const body = c.body ?? "";
            if (USAGE_LIMIT.test(body)) events.push({ at, order: 1, quota: c });
            else if (reviewedCommit(body)) events.push({ at, order: 1, names: reviewedCommit(body) });
        }
    }
    for (const r of reviews) {
        if (r.user?.login === CODEX_LOGIN && reviewedCommit(r.body)) {
            events.push({ at: String(r.submitted_at), order: 1, names: reviewedCommit(r.body) });
        }
    }
    // Requests sort before answers posted in the same second.
    events.sort((a, b) => a.at.localeCompare(b.at) || a.order - b.order);

    const attributed = new Map();
    let outstanding = [];
    for (const e of events) {
        if (e.request) outstanding.push(e.request);
        else if (e.names) outstanding = outstanding.filter((head) => !head.startsWith(e.names));
        else if (e.quota) {
            if (outstanding.length === 1) attributed.set(e.quota, outstanding[0]);
            outstanding = [];
        }
    }
    return attributed;
}

/** The head a ReAgent skip comment names, or null. */
export function skippedHead(comment) {
    return skipNote(comment)?.sha ?? null;
}

/** A ReAgent skip comment's { sha, reason }, or null. */
export function skipNote(comment) {
    if (comment?.user?.login !== SKIP_AUTHOR) return null;
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
 * ReAgent's quota skips, as `quota` outputs, oldest first. ReAgent edits one
 * comment in place per outage, so its last edit dates the skip of the head
 * it now names. Not Codex verdicts: see evaluateCodexGate for precedence.
 */
function reagentSkips(comments = []) {
    return comments
        .filter((c) => skipNote(c))
        .map((c) => ({ kind: "quota", reason: skipNote(c).reason, at: c.updated_at ?? c.created_at, sha: skipNote(c).sha }))
        .sort((a, b) => String(a.at).localeCompare(String(b.at)));
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
            .filter((c) => c.user?.login === CODEX_LOGIN)
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
            .filter((r) => r.user?.login === CODEX_LOGIN && r.state !== "DISMISSED")
            .map((r) => ({
                kind: "findings",
                at: r.submitted_at,
                sha: reviewedCommit(r.body),
                files: flagged.get(r.id) ?? new Set(),
            })),
    ]
        .filter((o) => o.sha !== null)
        .sort((a, b) => String(a.at).localeCompare(String(b.at)));
}

/** Codex's most recent verdict on any commit of the PR, or null. */
export function latestCodexOutput({ comments = [], reviews = [], reviewComments = [] }) {
    return codexOutputs({ comments, reviews, reviewComments }).at(-1) ?? null;
}

/**
 * Decide the status for `headSha` from the PR's issue comments, reviews and
 * review comments (GitHub REST shapes). The latest Codex output naming this
 * head wins, so a spontaneous findings review after an OK takes the OK back.
 * A ReAgent quota skip counts only when Codex has said nothing about the
 * head: ReAgent can post or edit it before reading Codex's answers, so its
 * date can trail a real verdict it must not override.
 *
 * With nothing on the head, Codex's latest verdict (never a skip) carries
 * over when it is an OK or findings only on docs, and `filesSinceLatest`
 * (the diff from its commit to the head, null if unknown) is docs-only.
 */
export function evaluateCodexGate({ headSha, comments = [], reviews = [], reviewComments = [], filesSinceLatest = null }) {
    const head = headSha.toLowerCase();
    const short = head.slice(0, 10);
    const all = codexOutputs({ comments, reviews, reviewComments });
    const namesHead = (o) => head.startsWith(o.sha);
    const onHead = all.filter(namesHead).at(-1) ?? reagentSkips(comments).filter(namesHead).at(-1);

    if (onHead?.kind === "ok") {
        return { state: "success", description: `Codex found no major issues in ${short}` };
    }
    if (onHead?.kind === "quota") {
        // A ReAgent skip names why it didn't ask; Codex's own notice is quota.
        if (onHead.reason === "round-cap") {
            return {
                state: "success",
                description: `ReAgent stopped asking Codex after this PR's round cap; ${short} passes without it ('@reagentx-workflow codex re-review' asks anyway)`,
            };
        }
        if (onHead.reason === "docs-only") {
            return { state: "success", description: `Docs-only PR: Codex isn't asked; ${short} passes without it` };
        }
        return { state: "success", description: `Codex is out of review quota; ${short} passes without it` };
    }
    if (isDocsOnlyFindings(onHead)) {
        return { state: "success", description: `Codex only flagged docs in ${short}; see its comments` };
    }
    if (onHead?.kind === "findings") {
        return {
            state: "failure",
            description: `Codex left findings on ${short}; ReAgent re-asks once a push touches a flagged file`,
        };
    }
    if (onHead) {
        return { state: "pending", description: `Codex commented on ${short} without an OK` };
    }

    const latest = all.at(-1);
    // ReAgent re-asks for any non-doc change after either verdict, so both
    // carry only across a docs-only diff; a code change waits for Codex.
    const onlyDocsSince = Array.isArray(filesSinceLatest) && filesSinceLatest.every(isDocsOnlyPath);
    if (latest?.kind === "ok" && onlyDocsSince) {
        return { state: "success", description: `Codex OK on ${latest.sha}; only docs changed since` };
    }
    if (isDocsOnlyFindings(latest) && onlyDocsSince) {
        return { state: "success", description: `Codex only flagged docs in ${latest.sha}; see its comments` };
    }
    return {
        state: "pending",
        description: `Waiting for Codex on ${short}: ReAgent asks after approving, or comment '@reagentx-workflow codex re-review'`,
    };
}

async function gh(path, token, init = {}) {
    const res = await fetch(`https://api.github.com${path}`, {
        ...init,
        headers: {
            Authorization: `Bearer ${token}`,
            Accept: "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
            ...(init.headers ?? {}),
        },
    });
    if (!res.ok) {
        throw new Error(`${init.method ?? "GET"} ${path} -> HTTP ${res.status}: ${(await res.text()).slice(0, 300)}`);
    }
    return res;
}

async function ghAll(path, token) {
    const items = [];
    for (let page = 1; ; page++) {
        const sep = path.includes("?") ? "&" : "?";
        const batch = await (await gh(`${path}${sep}per_page=100&page=${page}`, token)).json();
        items.push(...batch);
        if (batch.length < 100) return items;
    }
}

// The compare API lists at most 300 files; a list that long may be cut short.
const COMPARE_FILE_CAP = 300;

/** Files changed base...head, or null when that can't be known completely. */
async function changedFiles(repo, base, head, token) {
    try {
        const cmp = await (await gh(`/repos/${repo}/compare/${base}...${head}`, token)).json();
        const files = (cmp.files ?? []).map((f) => f.filename);
        return files.length >= COMPARE_FILE_CAP ? null : files;
    } catch (err) {
        // A force-push can orphan the reviewed commit; treat as unknown.
        console.log(`compare ${base}...${head.slice(0, 10)} failed: ${err.message}`);
        return null;
    }
}

async function main() {
    const { GITHUB_TOKEN: token, GITHUB_REPOSITORY: repo, PR_NUMBER: pr, DRY_RUN } = process.env;
    if (!token || !repo || !pr) {
        throw new Error("GITHUB_TOKEN, GITHUB_REPOSITORY and PR_NUMBER are required");
    }
    const pull = await (await gh(`/repos/${repo}/pulls/${pr}`, token)).json();
    const headSha = pull.head.sha;
    const [comments, reviews, reviewComments] = await Promise.all([
        ghAll(`/repos/${repo}/issues/${pr}/comments`, token),
        ghAll(`/repos/${repo}/pulls/${pr}/reviews`, token),
        // Inline comments: the files each findings review flagged.
        ghAll(`/repos/${repo}/pulls/${pr}/comments`, token),
    ]);
    // Only an OK or docs-only findings on an earlier commit can carry, so
    // only then is the diff worth fetching.
    const latest = latestCodexOutput({ comments, reviews, reviewComments });
    const filesSinceLatest =
        (latest?.kind === "ok" || isDocsOnlyFindings(latest)) && !headSha.toLowerCase().startsWith(latest.sha)
            ? await changedFiles(repo, latest.sha, headSha, token)
            : null;
    const result = evaluateCodexGate({ headSha, comments, reviews, reviewComments, filesSinceLatest });
    console.log(`#${pr} ${headSha.slice(0, 10)}: ${result.state} — ${result.description}`);
    if (DRY_RUN) return;
    await gh(`/repos/${repo}/statuses/${headSha}`, token, {
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
