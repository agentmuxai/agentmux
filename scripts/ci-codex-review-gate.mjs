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
//     limits for code reviews. ..." naming no commit. It answers the latest
//     ReAgent trigger ("@codex review" by a5af with
//     `<!-- reagent:codex-trigger head=<sha> -->`), so it counts for that
//     head, and passes it: Codex being unavailable must not block merges
//     (#3562 sat pending). A later real verdict on the head still wins.
//
// Codex only reviews when asked by a5af, not a bot. ReAgent decides when
// to ask (a5af/reagent lambdas/codex_policy.py): after it approves a head,
// then again only when a push touches a file Codex flagged, or when a
// write-access commenter says "@reagentx-workflow codex re-review". After an
// OK it does NOT re-ask for a docs-only diff, so this gate carries an OK
// across exactly that diff. This gate only reads.

export const CODEX_LOGIN = "chatgpt-codex-connector[bot]";
export const STATUS_CONTEXT = "Codex review";

// Only ReAgent's trigger names the head it asked about; reagent's
// codex_policy.py reads the same marker from the same author.
export const TRIGGER_AUTHOR = "a5af";

const REVIEWED_COMMIT = /Reviewed commit:\**\s*`([0-9a-f]{7,40})`/i;
// Straight or curly apostrophe.
const NO_MAJOR_ISSUES = /Didn.t find any major issues/i;
const USAGE_LIMIT = /reached your Codex usage limits/i;
const TRIGGER_HEAD = /reagent:codex-trigger\s+head=([0-9a-f]{7,40})/i;

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

/** The head named by the latest ReAgent trigger posted at or before `at`, or null. */
function triggeredHeadAt(comments, at) {
    let head = null;
    let headAt = "";
    for (const c of comments) {
        if (c.user?.login !== TRIGGER_AUTHOR || String(c.created_at) > String(at)) continue;
        const m = TRIGGER_HEAD.exec(c.body ?? "");
        if (m && String(c.created_at) >= headAt) {
            head = m[1].toLowerCase();
            headAt = String(c.created_at);
        }
    }
    return head;
}

function commentOutput(c, comments) {
    const body = c.body ?? "";
    if (USAGE_LIMIT.test(body)) {
        return { kind: "quota", at: c.created_at, sha: triggeredHeadAt(comments, c.created_at) };
    }
    return { kind: NO_MAJOR_ISSUES.test(body) ? "ok" : "other", at: c.created_at, sha: reviewedCommit(body) };
}

/** Every Codex verdict that names a commit, oldest first. */
function codexOutputs({ comments = [], reviews = [] }) {
    return [
        ...comments.filter((c) => c.user?.login === CODEX_LOGIN).map((c) => commentOutput(c, comments)),
        // A dismissed findings review no longer counts against a commit. It
        // does not count for it either: only a Codex OK passes.
        ...reviews
            .filter((r) => r.user?.login === CODEX_LOGIN && r.state !== "DISMISSED")
            .map((r) => ({ kind: "findings", at: r.submitted_at, sha: reviewedCommit(r.body) })),
    ]
        .filter((o) => o.sha !== null)
        .sort((a, b) => String(a.at).localeCompare(String(b.at)));
}

/** Codex's most recent verdict on any commit of the PR, or null. */
export function latestCodexOutput({ comments = [], reviews = [] }) {
    return codexOutputs({ comments, reviews }).at(-1) ?? null;
}

/**
 * Decide the status for `headSha` from the PR's issue comments and reviews
 * (GitHub REST shapes). The latest Codex output naming this head wins, so a
 * spontaneous findings review after an OK takes the OK back.
 *
 * With nothing on the head, an OK carries over when Codex's latest verdict
 * is that OK and `filesSinceLatest` (the diff from its commit to the head,
 * null if unknown) is docs-only.
 */
export function evaluateCodexGate({ headSha, comments = [], reviews = [], filesSinceLatest = null }) {
    const head = headSha.toLowerCase();
    const short = head.slice(0, 10);
    const all = codexOutputs({ comments, reviews });
    const onHead = all.filter((o) => head.startsWith(o.sha)).at(-1);

    if (onHead?.kind === "ok") {
        return { state: "success", description: `Codex found no major issues in ${short}` };
    }
    if (onHead?.kind === "quota") {
        return { state: "success", description: `Codex is out of review quota; ${short} passes without it` };
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
    if (latest?.kind === "ok" && Array.isArray(filesSinceLatest) && filesSinceLatest.every(isDocsOnlyPath)) {
        return { state: "success", description: `Codex OK on ${latest.sha}; only docs changed since` };
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
    const [comments, reviews] = await Promise.all([
        ghAll(`/repos/${repo}/issues/${pr}/comments`, token),
        ghAll(`/repos/${repo}/pulls/${pr}/reviews`, token),
    ]);
    // Only an OK on an earlier commit can carry, so only then is the diff worth fetching.
    const latest = latestCodexOutput({ comments, reviews });
    const filesSinceLatest =
        latest?.kind === "ok" && !headSha.toLowerCase().startsWith(latest.sha)
            ? await changedFiles(repo, latest.sha, headSha, token)
            : null;
    const result = evaluateCodexGate({ headSha, comments, reviews, filesSinceLatest });
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
