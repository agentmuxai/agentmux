// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Sets the `Codex review` commit status on a PR's head: success only once
// Codex has said "Didn't find any major issues" about that exact commit.
// Run by .github/workflows/codex-review-gate.yml.
//
// Why a status keyed to the head commit: #3513 merged at 18:07 while Codex
// was still finding real bugs in it; its OK came at 20:02, and three fixes
// were orphaned (#3538). A push after an OK needs a fresh OK, so the OK is
// matched on the "Reviewed commit" Codex prints, never on "Codex said OK
// somewhere in this PR".
//
// What Codex posts (observed on #3513/#3523/#3538):
//   - no findings: an ISSUE COMMENT "Codex Review: Didn't find any major
//     issues. ..." with "**Reviewed commit:** `<10-char sha>`"
//   - findings:    a PR REVIEW (state COMMENTED) with the same
//     "Reviewed commit" line and inline comments.
//
// Codex only reviews when asked by a5af, not a bot; reagent posts that
// request once per head commit (a5af/reagent#241). This gate only reads.

export const CODEX_LOGIN = "chatgpt-codex-connector[bot]";
export const STATUS_CONTEXT = "Codex review";

const REVIEWED_COMMIT = /Reviewed commit:\**\s*`([0-9a-f]{7,40})`/i;
// Straight or curly apostrophe.
const NO_MAJOR_ISSUES = /Didn.t find any major issues/i;

export function reviewedCommit(body) {
    const m = REVIEWED_COMMIT.exec(body ?? "");
    return m ? m[1].toLowerCase() : null;
}

/**
 * Decide the status for `headSha` from the PR's issue comments and reviews
 * (GitHub REST shapes). The latest Codex output naming this head wins, so a
 * spontaneous findings review after an OK takes the OK back.
 */
export function evaluateCodexGate({ headSha, comments = [], reviews = [] }) {
    const head = headSha.toLowerCase();
    const short = head.slice(0, 10);
    const outputs = [
        ...comments.map((c) => ({ kind: "comment", at: c.created_at, body: c.body, login: c.user?.login })),
        ...reviews.map((r) => ({ kind: "review", at: r.submitted_at, body: r.body, login: r.user?.login })),
    ]
        .filter((o) => o.login === CODEX_LOGIN)
        .filter((o) => {
            const sha = reviewedCommit(o.body);
            return sha !== null && head.startsWith(sha);
        })
        .sort((a, b) => String(a.at).localeCompare(String(b.at)));

    const latest = outputs.at(-1);
    if (!latest) {
        return { state: "pending", description: `Waiting for Codex to review ${short}` };
    }
    if (latest.kind === "comment" && NO_MAJOR_ISSUES.test(latest.body ?? "")) {
        return { state: "success", description: `Codex found no major issues in ${short}` };
    }
    if (latest.kind === "review") {
        return { state: "failure", description: `Codex left findings on ${short}; push a fix to get a fresh review` };
    }
    return { state: "pending", description: `Codex commented on ${short} without an OK` };
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
    const result = evaluateCodexGate({ headSha, comments, reviews });
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
