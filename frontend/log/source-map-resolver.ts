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
 * Resolve a single frame line. Returns the rewritten line, or the
 * original line if no resolution is possible right now.
 *
 * `consumer` is optional — when omitted, only frames whose chunk is
 * already in the synchronous-ready cache get resolved.
 */
function resolveFrameSync(line: string): { line: string; resolved: boolean } {
    const m = line.match(FRAME_RE);
    if (!m) return { line, resolved: false };
    const [, prefix, funcName, url, lineStr, colStr] = m;
    const chunk = chunkOf(url);
    const entry = cache.get(chunk);
    if (entry?.kind !== "ready") return { line, resolved: false };

    const pos = entry.consumer.originalPositionFor({
        line: Number(lineStr),
        column: Number(colStr),
    });
    if (pos.source == null || pos.line == null) return { line, resolved: false };

    const displayName = pos.name ?? funcName ?? "<anonymous>";
    return {
        line: `${prefix}${displayName} (${pos.source}:${pos.line}${pos.column != null ? `:${pos.column}` : ""})`,
        resolved: true,
    };
}

/**
 * Synchronous resolve: rewrite every frame whose chunk's map is
 * already cached. Returns the resolved stack + a `partial` flag
 * indicating whether any frame couldn't be resolved (caller can
 * then call {@link resolveStack} for the async follow-up).
 */
export function resolveStackSync(stack: string): {
    resolved: string;
    partial: boolean;
} {
    const lines = stack.split("\n");
    let anyUnresolved = false;
    let anyFrameTouched = false;
    const out = lines.map((line) => {
        if (!line.includes(" at ")) return line;
        anyFrameTouched = true;
        const { line: next, resolved } = resolveFrameSync(line);
        if (!resolved) anyUnresolved = true;
        return next;
    });
    return {
        resolved: out.join("\n"),
        // `partial` only matters when there was at least one frame
        // to look at. A stackless string returns partial: false.
        partial: anyFrameTouched && anyUnresolved,
    };
}

/**
 * Async resolve: ensures every chunk's `.map` is loaded (or marked
 * failed), then runs the synchronous resolver against the stack.
 * Used by the forwarder as a follow-up after the primary emit.
 */
export async function resolveStack(stack: string): Promise<string> {
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
    return resolveStackSync(stack).resolved;
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
