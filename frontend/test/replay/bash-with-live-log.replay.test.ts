// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * End-to-end replay test against a synthetic Bash + live-log fixture.
 *
 * What this proves:
 *   - The session-replay framework loads + validates an NDJSON fixture.
 *   - The replay driver demuxes stream-json + wps + dispatch events to
 *     the right reducer commands.
 *   - The agent-document reducer accumulates ToolChunkAppend events on
 *     the matching ToolNode (a smoke test for the live-log feature's
 *     state path; PR #800 / #815).
 *
 * Adding more fixtures here is the v1 expansion path — every recorded
 * bug repro becomes its own `*.replay.test.ts` with one assert per
 * regression we don't want to ship again. Spec:
 * `docs/specs/SPEC_AGENT_PANE_SESSION_REPLAY_2026_05_12.md`.
 */

import { describe, expect, it } from "vitest";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { loadFixture } from "./loader";
import { replayInstant } from "./replay";
import type { ToolNode } from "@/app/view/agent/types";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const FIXTURE = path.resolve(
    __dirname,
    "../fixtures/agent-sessions/bash-with-live-log.session.ndjson",
);

describe("agent-pane session replay: bash-with-live-log", () => {
    it("loads + parses the fixture", () => {
        const fixture = loadFixture(FIXTURE);
        expect(fixture.header.version).toBe(1);
        expect(fixture.header.provider).toBe("claude");
        expect(fixture.events.length).toBeGreaterThan(0);
        // Trailer has expectations the replay must satisfy.
        expect(fixture.trailer?.expect).toBeDefined();
    });

    it("applies stream-json + wps events through the real reducers", () => {
        const fixture = loadFixture(FIXTURE);
        const result = replayInstant(fixture);

        const expected = fixture.trailer!.expect as {
            tool_chunks_applied: number;
            warnings_count: number;
            tool_id: string;
            final_kind_breakdown: Record<string, number>;
        };

        // Tool chunks were dispatched (3 stdout + 1 terminal=>system).
        expect(result.stats.toolChunksApplied).toBe(expected.tool_chunks_applied);

        // No warnings — the chunks landed on a known tool, nothing dropped.
        expect(result.warnings).toEqual([]);
        expect(result.stats.eventsDropped).toBe(0);
        // (cross-check via fixture expectation too)
        expect(result.warnings.length).toBe(expected.warnings_count);

        // The ToolNode should have the live-log chunks attached.
        const toolNode = result.docState.nodes.find(
            (n): n is ToolNode => n.type === "tool" && n.id === expected.tool_id,
        );
        expect(toolNode).toBeDefined();
        const chunks = toolNode!.log?.chunks ?? [];
        expect(chunks.length).toBe(expected.tool_chunks_applied);

        // Kinds match the breakdown the fixture predicts.
        const breakdown: Record<string, number> = {};
        for (const c of chunks) breakdown[c.kind] = (breakdown[c.kind] ?? 0) + 1;
        expect(breakdown).toEqual(expected.final_kind_breakdown);

        // Content sanity — stdout chunks should appear in input order.
        const stdoutContents = chunks
            .filter((c) => c.kind === "stdout")
            .map((c) => c.content);
        expect(stdoutContents).toEqual(["file1.txt", "file2.txt", "file3.txt"]);
    });

    it("rejects fixtures with non-monotonic seq", () => {
        // Negative test for the loader's strict-shape guarantees.
        // Inline a small bad fixture to a temp file and assert throw.
        // Using a literal string avoids file I/O in the test happy path.
        const bad = [
            JSON.stringify({
                kind: "header",
                version: 1,
                agentmux_version: "0.33.817",
                schema_version: 8,
                recorded_at: "2026-05-12T15:00:00Z",
                provider: "claude",
                block_id: "x",
                instance_name: "x",
                redactions: [],
            }),
            JSON.stringify({
                seq: 5,
                t_ms: 0,
                src: "stream-json",
                line: "{}",
            }),
            // seq goes backwards — must throw.
            JSON.stringify({
                seq: 3,
                t_ms: 1,
                src: "stream-json",
                line: "{}",
            }),
        ].join("\n");

        const fs = require("node:fs") as typeof import("node:fs");
        const tmp = path.join(__dirname, `.tmp-bad-${Date.now()}.ndjson`);
        fs.writeFileSync(tmp, bad, "utf8");
        try {
            expect(() => loadFixture(tmp)).toThrow(/seq must be strictly increasing/);
        } finally {
            fs.unlinkSync(tmp);
        }
    });
});
