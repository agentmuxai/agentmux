// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A small GitHub REST client for CI scripts that retries the errors that clear
// on their own: 5xx, rate limits and dropped connections. Without it, a single
// HTTP 500 on a status POST left #4863's required check stuck for most of a day.
//
// Every method is retried. That is safe for today's callers because each write
// they make is idempotent in effect: a commit status is append-only and the
// latest per context wins, and a duplicate workflow_dispatch starts a run the
// per-PR concurrency group cancels. A caller with a write that must not repeat
// passes { retry: false }.
//
// docs/specs/PLAN_CODEX_GATE_TRANSIENT_FAILURES_AND_CI_SCRIPT_DRY_2026_10_14.md §3.1

const API = "https://api.github.com";
const RETRY_STATUSES = new Set([500, 502, 503, 504]);
const PAGE_SIZE = 100;

export class GitHubError extends Error {
    constructor(method, path, status, body) {
        super(`${method} ${path} -> HTTP ${status}: ${String(body ?? "").slice(0, 300)}`);
        this.name = "GitHubError";
        this.method = method;
        this.path = path;
        this.status = status;
    }
}

const defaultSleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** 2 s, 4 s, 8 s, … after the 1st, 2nd, 3rd failed attempt, plus up to 1 s of jitter. */
export function backoffMs(attempt, random = Math.random) {
    return 2000 * 2 ** (attempt - 1) + Math.floor(random() * 1000);
}

/**
 * How long to wait before retrying the failed response `res` (its `attempt`th
 * try), or null when it should not be retried. A 403 is retried only when
 * GitHub says it is a rate limit; otherwise it is a permission error.
 */
export function retryDelayMs(res, attempt, { now = Date.now(), random = Math.random } = {}) {
    if (RETRY_STATUSES.has(res.status)) return backoffMs(attempt, random);
    if (res.status !== 429 && res.status !== 403) return null;
    const retryAfter = res.headers.get("retry-after");
    if (retryAfter !== null && Number.isFinite(Number(retryAfter))) return Number(retryAfter) * 1000;
    if (res.headers.get("x-ratelimit-remaining") === "0") {
        const reset = Number(res.headers.get("x-ratelimit-reset"));
        // The reset is in whole epoch seconds; a second more lands after it.
        return Number.isFinite(reset) && reset > 0 ? Math.max(0, reset * 1000 - now) + 1000 : null;
    }
    return res.status === 429 ? backoffMs(attempt, random) : null;
}

/**
 * `request(path, init?, { retry? })` returns the ok Response; `json` its body;
 * `paginate(path)` every item of a list endpoint. A failure after the last
 * attempt throws a GitHubError (an HTTP error) or the fetch error (no response).
 */
export function createGitHubClient({
    token,
    fetch = globalThis.fetch,
    sleep = defaultSleep,
    log = console.log,
    now = Date.now,
    random = Math.random,
    maxAttempts = 4,
    maxWaitMs = 60_000,
} = {}) {
    async function request(path, init = {}, { retry = true } = {}) {
        const method = init.method ?? "GET";
        for (let attempt = 1; ; attempt++) {
            const last = !retry || attempt >= maxAttempts;
            let res;
            try {
                res = await fetch(`${API}${path}`, {
                    ...init,
                    headers: {
                        Authorization: `Bearer ${token}`,
                        Accept: "application/vnd.github+json",
                        "X-GitHub-Api-Version": "2022-11-28",
                        ...(init.headers ?? {}),
                    },
                });
            } catch (err) {
                // fetch rejects only when no response arrived (DNS, reset, timeout).
                if (last) throw new Error(`${method} ${path} failed: ${err.message}`, { cause: err });
                await wait(method, path, err.message, attempt, backoffMs(attempt, random));
                continue;
            }
            if (res.ok) return res;
            const delay = last ? null : retryDelayMs(res, attempt, { now: now(), random });
            if (delay === null || delay > maxWaitMs) {
                throw new GitHubError(method, path, res.status, await res.text().catch(() => ""));
            }
            // Free the connection; the body of a response we retry is never read.
            await res.body?.cancel().catch(() => {});
            await wait(method, path, `HTTP ${res.status}`, attempt, delay);
        }
    }

    async function wait(method, path, why, attempt, ms) {
        log(`${method} ${path} -> ${why}; retry ${attempt + 1}/${maxAttempts} in ${(ms / 1000).toFixed(1)}s`);
        await sleep(ms);
    }

    async function json(path, init, opts) {
        return (await request(path, init, opts)).json();
    }

    async function paginate(path) {
        const items = [];
        const sep = path.includes("?") ? "&" : "?";
        for (let page = 1; ; page++) {
            const batch = await json(`${path}${sep}per_page=${PAGE_SIZE}&page=${page}`);
            items.push(...batch);
            if (batch.length < PAGE_SIZE) return items;
        }
    }

    return { request, json, paginate };
}
