// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The API block's GitHub presets (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.5): not a new kind of
 * block, an API node with its method, URL, headers and body filled in for a common GitHub read or write. The
 * person picks a preset and a repository, and the node is an ordinary API node they can still edit.
 *
 * Every URL names `api.github.com` in full, which is what the Vault credential is bound to: the key is sent to
 * that one site and the first call asks the person. A write preset (`writes`) says so on its row, because it
 * changes something outside AgentMux.
 *
 * The ids in a URL are `{{…}}` references the person points at their trigger's fields; a preset never fills in
 * a value of its own. A comment's text goes through `jsonstr`, so text from outside (a log, a pull request body)
 * cannot close the JSON string and add fields of its own.
 */

export interface GithubPreset {
    id: string;
    label: string;
    method: "GET" | "POST" | "PUT";
    /** The path after `/repos/<owner>/<name>`. */
    path: string;
    /** A JSON body template, for a write that has one. */
    body?: string;
    /** Changes something on GitHub, not only reads it. */
    writes: boolean;
    /** One line on what it returns or does. */
    hint: string;
}

export const GITHUB_API = "https://api.github.com";

/** The headers every GitHub preset sends. GitHub refuses a request with no User-Agent. */
export const GITHUB_HEADERS: Record<string, string> = {
    Accept: "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
    "User-Agent": "AgentMux-Drone",
};

export const GITHUB_PRESETS: readonly GithubPreset[] = [
    {
        id: "runs",
        label: "Workflow runs (latest)",
        path: "/actions/runs?per_page=20",
        method: "GET",
        writes: false,
        hint: "The latest runs; add &status=completed, &event=schedule or &branch=main to the URL to narrow it.",
    },
    {
        id: "run",
        label: "A workflow run",
        path: "/actions/runs/{{trigger.run_id}}",
        method: "GET",
        writes: false,
        hint: "One run: its conclusion, branch, commit and attempt.",
    },
    {
        id: "run-jobs",
        label: "A run's jobs",
        path: "/actions/runs/{{trigger.run_id}}/jobs",
        method: "GET",
        writes: false,
        hint: "Each job of the run with its steps; body.jobs lists them.",
    },
    {
        id: "job-log-link",
        label: "A job's log (step 1: the link)",
        path: "/actions/jobs/{{trigger.job_id}}/logs",
        method: "GET",
        writes: false,
        hint: "GitHub answers 302 with the log's address in headers.location. A call with a credential follows no redirect, so read that address with a second API node that has no credential, and tail it ({{log.body | tail:200}}).",
    },
    {
        id: "rerun-failed",
        label: "Re-run failed jobs",
        path: "/actions/runs/{{trigger.run_id}}/rerun-failed-jobs",
        method: "POST",
        writes: true,
        hint: "Starts the failed jobs of a run again. The run id is a number, so the URL stays typed.",
    },
    {
        id: "pull",
        label: "A pull request",
        path: "/pulls/{{trigger.number}}",
        method: "GET",
        writes: false,
        hint: "One pull request: its state, merged flag, author and body (the body is text from outside).",
    },
    {
        id: "pulls-closed",
        label: "Recently closed pull requests",
        path: "/pulls?state=closed&sort=updated&direction=desc&per_page=30",
        method: "GET",
        writes: false,
        hint: "Each has merged_at, which is empty for one closed without a merge.",
    },
    {
        id: "releases",
        label: "Releases (latest)",
        path: "/releases?per_page=10",
        method: "GET",
        writes: false,
        hint: "The latest releases with their tag and assets.",
    },
    {
        id: "comment",
        label: "Comment on a pull request or issue",
        path: "/issues/{{trigger.number}}/comments",
        method: "POST",
        body: '{"body": "{{text | jsonstr}}"}',
        writes: true,
        hint: "Replace `text` with the reference that holds the comment. jsonstr keeps it one JSON string.",
    },
    {
        id: "merge",
        label: "Merge a pull request",
        path: "/pulls/{{trigger.number}}/merge",
        method: "PUT",
        body: '{"merge_method": "squash"}',
        writes: true,
        hint: "Squash-merges it. GitHub refuses it while its checks or reviews are not met.",
    },
];

/** `owner/name`, as GitHub spells a repository; nothing else is let into a URL. */
const REPO = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;

export function isGithubRepo(repo: string): boolean {
    const r = repo.trim();
    return REPO.test(r) && r !== "./." && !r.split("/").some((part) => part === "." || part === "..");
}

export function githubPresetById(id: string): GithubPreset | undefined {
    return GITHUB_PRESETS.find((p) => p.id === id);
}

/**
 * The node-data patch that fills an API node with a preset for `repo`. `null` for a repository that is not
 * `owner/name`. It leaves `auth` as the node has it: the credential is the person's to choose.
 */
export function githubPresetPatch(preset: GithubPreset, repo: string): Record<string, unknown> | null {
    if (!isGithubRepo(repo)) return null;
    return {
        method: preset.method,
        url: `${GITHUB_API}/repos/${repo.trim()}${preset.path}`,
        headers: { ...GITHUB_HEADERS, ...(preset.body ? { "Content-Type": "application/json" } : {}) },
        body: preset.body ?? "",
    };
}

/** The repository a node's URL already names, so the picker can start from it. */
export function githubRepoOf(url: string): string {
    const m = /^https:\/\/api\.github\.com\/repos\/([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)(?:[/?]|$)/.exec(url.trim());
    return m ? m[1] : "";
}

/** The words of the preset picker. Kept here, not in the JSX, until the drone view's text moves into the dictionary. */
export const PRESET_TEXT = {
    repoLabel: "GitHub repo",
    repoPlaceholder: "owner/name",
    presetLabel: "GitHub preset",
    fill: "Fill from a preset…",
    needRepo: "Give the repository as owner/name first.",
    writesNote: "Changes something on GitHub. ",
} as const;

/** A preset's name in the list; a preset that changes something on GitHub says so. */
export function presetOptionLabel(g: Pick<GithubPreset, "label" | "writes">): string {
    return g.writes ? `${g.label} (writes)` : g.label;
}
