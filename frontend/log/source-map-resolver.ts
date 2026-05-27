// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Runtime source-map resolver for piped error stacks.
 *
 * V8 (Chromium) only applies source maps to stacks displayed in
 * DevTools. The runtime `error.stack` string is always the minified
 * bundle position — `at bpe (http://.../assets/index-Hash.js:3:31187)`.
 * That string is what `frontend/log/error-forwarder.ts` pipes to the
 * host via `fe_log_structured`, which means every renderer crash in
 * the host log requires a manual DevTools dance to decode.
 *
 * This module flips that: on first error in a bundle chunk it
 * lazily fetches the chunk's `.map`, parses it with `source-map-js`,
 * caches the `SourceMapConsumer`, and rewrites each frame to the
 * original `source:line:column (name)`. Subsequent errors in the
 * same chunk are synchronous lookups.
 *
 * Spec: docs/specs/SPEC_FE_SOURCE_MAP_RESOLVER_2026_05_27.md.
 */

import { SourceMapConsumer, type RawSourceMap } from "source-map-js";

/** A chunk's parsed map, an in-flight fetch promise, or a failure sentinel. */
type CacheEntry =
    | { kind: "ready"; consumer: SourceMapConsumer }
    | { kind: "pending"; promise: Promise<SourceMapConsumer | "failed"> }
    | { kind: "failed" };

const cache = new Map<string, CacheEntry>();

/** Reported once per failing chunk to keep host-log noise low. */
const reportedFailures = new Set<string>();

/**
 * Match V8's stack frame format. Two shapes:
 *   `    at funcName (url:line:col)`
 *   `    at url:line:col` (anonymous, e.g. eval / top-level)
 * Spaces are flexible; the trailing nothing-or-newline is implicit.
 *
 * Capture groups: 1=funcName (may be empty), 2=url, 3=line, 4=col.
 */
const FRAME_RE = /^(\s*at\s+)(?:(.+?)\s+)?\(?([^()\s]+):(\d+):(\d+)\)?\s*$/;

/** Strip the URL down to its chunk filename. */
function chunkOf(url: string): string {
    // Match the last "/" -delimited segment that ends in `.js` (with
    // optional query string). Bare or relative URLs pass through.
    const m = url.match(/\/([^/]+\.js)(?:\?.*)?$/);
    return m ? m[1] : url;
}

/** Build the `.map` URL from a `.js` URL by appending `.map`. */
function mapUrlOf(jsUrl: string): string {
    return jsUrl.replace(/(\?.*)?$/, ".map");
}

/**
 * Async load + parse a `.map` file for a chunk. Caches the result
 * (success or failure) so retries are idempotent.
 */
function loadMap(jsUrl: string): Promise<SourceMapConsumer | "failed"> {
    const chunk = chunkOf(jsUrl);
    const entry = cache.get(chunk);
    if (entry?.kind === "ready") return Promise.resolve(entry.consumer);
    if (entry?.kind === "failed") return Promise.resolve("failed");
    if (entry?.kind === "pending") return entry.promise;

    const promise = (async (): Promise<SourceMapConsumer | "failed"> => {
        try {
            const res = await fetch(mapUrlOf(jsUrl));
            if (!res.ok) throw new Error(`HTTP ${res.status}`);
            const json = (await res.json()) as RawSourceMap;
            const consumer = new SourceMapConsumer(json);
            cache.set(chunk, { kind: "ready", consumer });
            return consumer;
        } catch (err) {
            cache.set(chunk, { kind: "failed" });
            if (!reportedFailures.has(chunk)) {
                reportedFailures.add(chunk);
                // One-shot WARN; the consumer of this module (the
                // error forwarder) typically forwards via the same
                // log pipe — that's fine, this isn't an error path
                // that recurses through resolveStack.
                console.warn(
                    `[source-map] missing or invalid for chunk ${chunk}: ${(err as Error).message}`,
                );
            }
            return "failed";
        }
    })();

    cache.set(chunk, { kind: "pending", promise });
    return promise;
}

/**
 * Resolve a single frame line. Returns the rewritten line and a status:
 *   - `resolved` — frame was rewritten against a ready consumer.
 *   - `pending`  — chunk's map isn't loaded yet; async path can retry.
 *   - `failed`   — chunk's map is permanently unavailable; do NOT retry.
 *   - `not-a-frame` — line didn't match the V8 frame regex.
 *
 * Callers (see `resolveStackSync`) treat `pending` as a reason to fire
 * the async follow-up, but treat `failed` as terminal — retrying buys
 * nothing and produces duplicate `(stack-resolved)` log lines for the
 * same raw stack (codex P2 on PR #1090).
 */
type FrameStatus = "resolved" | "pending" | "failed" | "not-a-frame";

function resolveFrameSync(line: string): { line: string; status: FrameStatus } {
    const m = line.match(FRAME_RE);
    if (!m) return { line, status: "not-a-frame" };
    const [, prefix, funcName, url, lineStr, colStr] = m;
    const chunk = chunkOf(url);
    const entry = cache.get(chunk);
    if (entry == null || entry.kind === "pending") return { line, status: "pending" };
    if (entry.kind === "failed") return { line, status: "failed" };

    // V8's `error.stack` columns are 1-based; source-map-js's
    // `originalPositionFor` expects 0-based generated columns. Without
    // the `-1`, every lookup shifts one character right and at token
    // boundaries can map to the wrong identifier — defeating the
    // purpose of the resolver (codex P2 on PR #1090). Floor at 0 to
    // tolerate any stack format that already gives a 0 column.
    const pos = entry.consumer.originalPositionFor({
        line: Number(lineStr),
        column: Math.max(0, Number(colStr) - 1),
    });
    if (pos.source == null || pos.line == null) return { line, status: "failed" };

    const displayName = pos.name ?? funcName ?? "<anonymous>";
    return {
        line: `${prefix}${displayName} (${pos.source}:${pos.line}${pos.column != null ? `:${pos.column}` : ""})`,
        status: "resolved",
    };
}

/**
 * Synchronous resolve: rewrite every frame whose chunk's map is
 * already cached. Returns the resolved stack + a `partial` flag
 * indicating whether any frame couldn't be resolved (caller can
 * then call {@link resolveStack} for the async follow-up).
 */
/**
 * Outcome of a stack resolve.
 *   - `"resolved"` — every frame was rewritten (or there were no
 *     frames). Caller can trust the resolved string as fully decoded.
 *   - `"partial"` — at least one frame's chunk is pending an async
 *     map load. Caller should fire the async follow-up.
 *   - `"failed"`  — at least one frame's chunk is terminally failed
 *     (404, malformed, or in-map miss). Async retry buys nothing;
 *     the resolved string still contains raw minified positions.
 *
 * Priority: pending dominates failed dominates resolved — so a stack
 * with both pending and failed frames reports `"partial"` (the async
 * follow-up will eventually re-report as `"failed"` once the pending
 * loads finish).
 */
export type ResolveStatus = "resolved" | "partial" | "failed";

export function resolveStackSync(stack: string): {
    resolved: string;
    status: ResolveStatus;
} {
    const lines = stack.split("\n");
    let anyPending = false;
    let anyFailed = false;
    const out = lines.map((line) => {
        if (!line.includes(" at ")) return line;
        const { line: next, status } = resolveFrameSync(line);
        // Track "needs async retry" (pending) separately from
        // "tried and gave up" (failed). Conflating them either
        // re-runs the resolver for nothing or — if we ignored
        // failure — falsely labels a still-raw stack as fully
        // resolved (codex P2 on PR #1090 b80a2ed6).
        if (status === "pending") anyPending = true;
        else if (status === "failed") anyFailed = true;
        return next;
    });
    return {
        resolved: out.join("\n"),
        status: anyPending ? "partial" : anyFailed ? "failed" : "resolved",
    };
}

/**
 * Async resolve: ensures every chunk's `.map` is loaded (or marked
 * failed), then runs the synchronous resolver against the stack.
 * Used by the forwarder as a follow-up after the primary emit.
 *
 * After awaiting every load, no chunk is `pending` — so the returned
 * status is always `"resolved"` or `"failed"`, never `"partial"`.
 */
export async function resolveStack(stack: string): Promise<{
    resolved: string;
    status: ResolveStatus;
}> {
    const lines = stack.split("\n");
    const urls = new Set<string>();
    for (const line of lines) {
        const m = line.match(FRAME_RE);
        if (m) urls.add(m[3]);
    }
    // Kick off all map loads in parallel; ignore individual failures —
    // resolveFrameSync handles the "failed" sentinel by passing the
    // raw line through.
    await Promise.allSettled(Array.from(urls).map(loadMap));
    return resolveStackSync(stack);
}

/** Test-only helper. Clears all cached maps + failure marks. */
export function _resetCacheForTests(): void {
    cache.clear();
    reportedFailures.clear();
}

/**
 * Test-only helper. Seeds a consumer for a chunk so the synchronous
 * path can resolve frames without a real fetch.
 */
export function _seedConsumerForTests(
    chunk: string,
    consumer: SourceMapConsumer,
): void {
    cache.set(chunk, { kind: "ready", consumer });
}
