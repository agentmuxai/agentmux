// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SourceMapConsumer, type RawSourceMap } from "source-map-js";

import {
    _resetCacheForTests,
    _seedConsumerForTests,
    resolveStack,
    resolveStackSync,
} from "./source-map-resolver";

/**
 * Build a SourceMapConsumer from a pre-encoded map literal. source-map-js
 * doesn't include a generator API, so we hand-author a tiny map with one
 * mapping point: generated line 3 col 0 → orig.ts line 11 col 5 (name "myFn").
 *
 * The version field is the literal number `3` rather than a typed `number`;
 * `RawSourceMap.version` is typed `3` (literal) in source-map-js, which is
 * why we use a cast through `as RawSourceMap` instead of an explicit type
 * annotation on the object literal.
 */
async function buildConsumer(): Promise<SourceMapConsumer> {
    // source-map-js's `.d.ts` types `version` as `string`, but the runtime
    // accepts the canonical number `3` from the source-map v3 spec. We cast
    // through `unknown` to keep the runtime value correct without lying
    // about the field's nominal type.
    const raw = {
        version: 3,
        file: "index-Hash.js",
        sources: ["src/orig.ts"],
        sourcesContent: ["function myFn() { /* original */ }\n"],
        names: ["myFn"],
        // Three generated lines (two `;;`), one segment on line 3:
        //   gen line 3, col 0 → orig.ts:11:5 (name myFn).
        // VLQ encoding of [0, 0, 10, 5, 0] → "AAUKA".
        mappings: ";;AAUKA",
    } as unknown as RawSourceMap;
    return new SourceMapConsumer(raw);
}

describe("source-map-resolver", () => {
    beforeEach(() => {
        _resetCacheForTests();
    });

    afterEach(() => {
        vi.restoreAllMocks();
    });

    it("resolveStackSync rewrites a frame whose map is seeded", async () => {
        const consumer = await buildConsumer();
        _seedConsumerForTests("index-Hash.js", consumer);

        const rawStack = [
            "NotFoundError: kaboom",
            "    at bpe (http://127.0.0.1:50000/assets/index-Hash.js:3:0)",
        ].join("\n");

        const { resolved, partial } = resolveStackSync(rawStack);
        expect(partial).toBe(false);
        expect(resolved).toContain("src/orig.ts:11");
        // Name from the map (myFn) overrides the runtime funcName (bpe).
        expect(resolved).toContain("myFn");
    });

    it("resolveStackSync leaves the line raw when the map isn't cached", () => {
        const rawStack = [
            "Error: ok",
            "    at fn (http://127.0.0.1:50000/assets/unknown-Hash.js:3:0)",
        ].join("\n");

        const { resolved, partial } = resolveStackSync(rawStack);
        expect(partial).toBe(true);
        expect(resolved).toContain("unknown-Hash.js");
    });

    it("resolveStackSync passes non-frame lines through unchanged", async () => {
        const consumer = await buildConsumer();
        _seedConsumerForTests("index-Hash.js", consumer);

        const raw = "TypeError: bang";
        const { resolved, partial } = resolveStackSync(raw);
        expect(resolved).toBe(raw);
        // No frames touched ⇒ no partial flag.
        expect(partial).toBe(false);
    });

    it("resolveStack fetches missing maps async, then resolves", async () => {
        const consumer = await buildConsumer();
        const fetchMock = vi.fn(async (url: string) => {
            expect(url).toContain("index-Hash.js.map");
            return {
                ok: true,
                json: async () =>
                    ({
                        version: 3,
                        file: "index-Hash.js",
                        sources: ["src/orig.ts"],
                        names: ["myFn"],
                        mappings: ";;AAUKA",
                    } as unknown as RawSourceMap),
            } as Response;
        });
        vi.stubGlobal("fetch", fetchMock);

        const rawStack = [
            "Error: x",
            "    at bpe (http://127.0.0.1:50000/assets/index-Hash.js:3:0)",
        ].join("\n");

        const out = await resolveStack(rawStack);
        // Source-map-js may or may not honor names depending on the
        // exact encoding — assert on the source position, which is
        // the load-bearing part.
        expect(out).toContain("src/orig.ts");
        expect(out).toMatch(/src\/orig\.ts:1[0-9]/);
        // Consumer is now silently used; suppress unused-warning.
        void consumer;
    });

    it("a missing .map (404) is cached as failed; no re-fetch on second call", async () => {
        const fetchMock = vi.fn(async () => ({
            ok: false,
            status: 404,
        }) as Response);
        vi.stubGlobal("fetch", fetchMock);
        // Also stub console.warn so the one-time WARN doesn't pollute
        // the test runner; we just verify the call count.
        const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => undefined);

        const stack = "Error: y\n    at f (http://x/assets/gone-Hash.js:1:0)";

        const r1 = await resolveStack(stack);
        const r2 = await resolveStack(stack);

        expect(fetchMock).toHaveBeenCalledTimes(1);
        expect(r1).toContain("gone-Hash.js");
        expect(r2).toContain("gone-Hash.js");
        // Single one-shot WARN per failing chunk.
        expect(warnSpy).toHaveBeenCalledTimes(1);
    });

    it("frames that don't match V8 format pass through unchanged", () => {
        const stack = [
            "Error: malformed",
            "    weird frame [no parens]",
            "    at fn (foo.js:1:1)",
        ].join("\n");
        const { resolved } = resolveStackSync(stack);
        // Middle line is non-conforming; we keep it as-is.
        expect(resolved).toContain("weird frame [no parens]");
    });

    it("converts V8's 1-based column to 0-based before source-map lookup", async () => {
        // Codex P2 on PR #1090: V8 `error.stack` columns are 1-based,
        // source-map-js `originalPositionFor` expects 0-based. Without
        // the conversion, every lookup shifts one character right.
        // Strategy: spy on the consumer to capture the column actually
        // passed into `originalPositionFor`.
        const consumer = await buildConsumer();
        const opfSpy = vi.spyOn(consumer, "originalPositionFor");
        _seedConsumerForTests("index-Hash.js", consumer);

        const stack = [
            "Error: c",
            // V8 reports col 7 (1-based). Resolver must call into
            // source-map-js with column 6 (0-based).
            "    at bpe (http://x/assets/index-Hash.js:3:7)",
        ].join("\n");
        resolveStackSync(stack);

        expect(opfSpy).toHaveBeenCalledWith({ line: 3, column: 6 });

        // Edge: col 0 must not become -1; floor at 0.
        opfSpy.mockClear();
        resolveStackSync("Error: c\n    at f (http://x/assets/index-Hash.js:3:0)");
        expect(opfSpy).toHaveBeenCalledWith({ line: 3, column: 0 });
    });

    it("failed chunks are terminal — resolveStackSync returns partial: false", async () => {
        // Codex P2 on PR #1090: after a chunk's `.map` is marked
        // failed, the sync resolver must NOT treat its frames as
        // pending — otherwise the forwarder fires another async
        // follow-up that re-runs the same failed lookup and emits a
        // duplicate `(stack-resolved)` log line with the still-raw
        // stack. The failure cache must be terminal.
        vi.stubGlobal("fetch", vi.fn(async () => ({ ok: false, status: 404 }) as Response));
        vi.spyOn(console, "warn").mockImplementation(() => undefined);

        const stack = "Error: z\n    at f (http://x/assets/gone-Hash.js:1:0)";

        // First trip: async load fails; chunk cached as failed.
        await resolveStack(stack);

        // Second trip: pure sync — chunk should be terminal-failed,
        // NOT counted as partial.
        const { partial } = resolveStackSync(stack);
        expect(partial).toBe(false);
    });
});
