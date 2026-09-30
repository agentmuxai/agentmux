// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Shared plumbing for every muxsh-family Node CLI (muxspect, muxopen, muxsh,
// and future additions — see
// docs/specs/SPEC_MUXSH_FULL_COLLECTION_2026_09_16.md §2.8): reading the
// $AGENTMUX_LOCAL_URL/$AGENTMUX_AUTH_KEY pair every authenticated core needs,
// and building the authenticated fetch() call every core makes against
// agentmux-srv's REST API.
//
// Deliberately does NOT read the response body. Each existing tool's
// error-handling strategy differs in ways worth preserving exactly (some
// read .json() on success and .text() on failure; some read .json() always
// with a {} fallback on parse failure) — unifying that part would risk
// silently changing what each tool prints on a real failure. This module
// only shares the part that was genuinely byte-identical everywhere: env
// lookup and the fetch() call itself.

/** Read the auth pair from the environment. Pure — no validation, no I/O, no
 * process.exit. Callers decide what "missing" means for their own error
 * message and exit code (wording has legitimately differed per tool; this
 * doesn't force it to converge). */
export function readAgentmuxEnv(env = process.env) {
    return { url: env.AGENTMUX_LOCAL_URL, authKey: env.AGENTMUX_AUTH_KEY };
}

/** Perform an authenticated request against agentmux-srv's REST API.
 *
 * Returns `{ resp, ok, status }` — the raw `Response` plus convenience
 * fields — without reading the body, so callers remain free to call
 * `.json()` or `.text()` exactly as they already do. Throws on network
 * failure (DNS, connection refused, etc.); every existing caller already
 * wraps this in its own try/catch with its own fail() wording.
 */
export async function agentmuxFetch(url, authKey, path, { method = "GET", body } = {}) {
    const headers = { "X-AuthKey": authKey };
    const init = { method, headers };
    if (body !== undefined) {
        headers["Content-Type"] = "application/json";
        init.body = JSON.stringify(body);
    }
    const resp = await fetch(`${url.replace(/\/$/, "")}${path}`, init);
    return { resp, ok: resp.ok, status: resp.status };
}
