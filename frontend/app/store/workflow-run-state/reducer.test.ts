// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { update } from "./reducer";
import { initialState, parseBlockOutput } from "./types";

describe("workflow-run-state reducer", () => {
    describe("RunStarted", () => {
        it("flips status to running and clears folded cells", () => {
            const seed = {
                ...initialState(),
                blockResults: { b1: { response: "stale" } },
                output: "stale output",
                error: "stale error",
            };
            const r = update(seed, {
                type: "RunStarted",
                runId: "r1",
                workflowId: "wf1",
            });
            expect(r.state.runId).toBe("r1");
            expect(r.state.workflowId).toBe("wf1");
            expect(r.state.status).toBe("running");
            expect(r.state.blockResults).toEqual({});
            expect(r.state.output).toBe("");
            expect(r.state.error).toBe("");
            expect(r.events).toEqual([
                { type: "run-started", runId: "r1", workflowId: "wf1" },
            ]);
        });
    });

    describe("BlockDone", () => {
        it("folds agent block output (response + cost_usd)", () => {
            const r = update(initialState(), {
                type: "BlockDone",
                blockId: "agent-1",
                output: { response: "hello", cost_usd: 0.0123, tokens: {} },
            });
            expect(r.state.blockResults["agent-1"]).toEqual({
                response: "hello",
                costUsd: 0.0123,
            });
            expect(r.events[0]).toMatchObject({
                type: "block-done",
                blockId: "agent-1",
            });
        });

        it("falls back to JSON-stringify for non-agent blocks", () => {
            const r = update(initialState(), {
                type: "BlockDone",
                blockId: "api-1",
                output: { status: 200, body: "{}" },
            });
            expect(r.state.blockResults["api-1"].response).toContain('"status":200');
        });

        it("overwrites prior result on the same blockId", () => {
            const r1 = update(initialState(), {
                type: "BlockDone",
                blockId: "agent-1",
                output: { response: "first" },
            });
            const r2 = update(r1.state, {
                type: "BlockDone",
                blockId: "agent-1",
                output: { response: "second" },
            });
            expect(r2.state.blockResults["agent-1"].response).toBe("second");
        });
    });

    describe("BlockError", () => {
        it("stores error string with empty response", () => {
            const r = update(initialState(), {
                type: "BlockError",
                blockId: "agent-1",
                error: "boom",
            });
            expect(r.state.blockResults["agent-1"]).toEqual({
                response: "",
                error: "boom",
            });
        });
    });

    describe("RunDone / RunFailed", () => {
        it("RunDone sets status=done and preserves blockResults", () => {
            const seeded = update(initialState(), {
                type: "BlockDone",
                blockId: "a",
                output: { response: "ok" },
            }).state;
            const r = update(seeded, { type: "RunDone", output: "final" });
            expect(r.state.status).toBe("done");
            expect(r.state.output).toBe("final");
            expect(r.state.blockResults["a"].response).toBe("ok");
        });

        it("RunFailed sets status=failed with error message", () => {
            const r = update(initialState(), {
                type: "RunFailed",
                error: "exec failed",
            });
            expect(r.state.status).toBe("failed");
            expect(r.state.error).toBe("exec failed");
        });

        it("RunDone with object output is JSON-stringified", () => {
            const r = update(initialState(), {
                type: "RunDone",
                output: { final: true },
            });
            expect(r.state.output).toBe('{"final":true}');
        });
    });

    describe("BackfilledFromRow", () => {
        it("populates blockResults from done + error rows; skips others", () => {
            const r = update(initialState(), {
                type: "BackfilledFromRow",
                runId: "r1",
                workflowId: "wf1",
                status: "done",
                output: "final",
                error: "",
                blocks: [
                    { blockId: "a", status: "done", output: { response: "hi" } },
                    { blockId: "b", status: "error", error: "bad" },
                    { blockId: "c", status: "running" },
                    { blockId: "d", status: "skipped" },
                ],
            });
            expect(Object.keys(r.state.blockResults).sort()).toEqual(["a", "b"]);
            expect(r.state.blockResults["a"].response).toBe("hi");
            expect(r.state.blockResults["b"].error).toBe("bad");
            expect(r.state.status).toBe("done");
            expect(r.state.output).toBe("final");
        });

        it("overwrites prior incremental blockResults — backfill is authoritative", () => {
            const seeded = update(initialState(), {
                type: "BlockDone",
                blockId: "a",
                output: { response: "stale" },
            }).state;
            const r = update(seeded, {
                type: "BackfilledFromRow",
                runId: "r1",
                workflowId: "wf1",
                status: "done",
                output: "",
                error: "",
                blocks: [{ blockId: "a", status: "done", output: { response: "fresh" } }],
            });
            expect(r.state.blockResults["a"].response).toBe("fresh");
        });

        it("is a no-op when blocks is empty and status already matches", () => {
            const seeded = { ...initialState(), status: "done" as const };
            const r = update(seeded, {
                type: "BackfilledFromRow",
                runId: "r1",
                workflowId: "wf1",
                status: "done",
                output: "",
                error: "",
                blocks: [],
            });
            expect(r.state).toBe(seeded);
            expect(r.events).toEqual([]);
        });
    });

    describe("Reset", () => {
        it("returns to initialState while preserving closed flag", () => {
            const seeded = {
                ...initialState(),
                runId: "r1",
                status: "running" as const,
                blockResults: { a: { response: "x" } },
            };
            const r = update(seeded, { type: "Reset" });
            expect(r.state).toEqual(initialState());
        });
    });

    describe("Disposed gate", () => {
        it("Disposed sets closed=true", () => {
            const r = update(initialState(), { type: "Disposed" });
            expect(r.state.closed).toBe(true);
        });

        it("Disposed is idempotent", () => {
            const r1 = update(initialState(), { type: "Disposed" });
            const r2 = update(r1.state, { type: "Disposed" });
            expect(r2.state).toBe(r1.state);
            expect(r2.events).toEqual([]);
        });

        it("commands after Disposed emit post-close-command-dropped and don't mutate state", () => {
            const closed = update(initialState(), { type: "Disposed" }).state;
            const r = update(closed, {
                type: "BlockDone",
                blockId: "a",
                output: {},
            });
            expect(r.state).toBe(closed);
            expect(r.events).toEqual([
                { type: "post-close-command-dropped", commandType: "BlockDone" },
            ]);
        });
    });

    describe("parseBlockOutput", () => {
        it("handles primitive string output", () => {
            expect(parseBlockOutput("hello").response).toBe("hello");
        });

        it("handles null/undefined safely", () => {
            expect(parseBlockOutput(null).response).toBe("null");
            expect(parseBlockOutput(undefined).response).toBe("null");
        });

        it("skips cost_usd when missing or non-number", () => {
            expect(
                parseBlockOutput({ response: "x", cost_usd: "0.1" }).costUsd,
            ).toBeUndefined();
            expect(parseBlockOutput({ response: "x" }).costUsd).toBeUndefined();
        });
    });
});
