// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for muxspect.mjs's argument parsing — pure, no network, no
// running instance needed. Runs as part of `npm test` (vitest) — NOT
// node:test (reagent P1 on PR #2380: vitest.config.ts's default include
// glob picks up this path, unlike tools/tests/lib/*.test.mjs which is
// excluded via `**/tools/**`; a node:test file here made vitest fail with
// "No test suite found in file" instead of actually running).
//
// Pins the fix for reagent P2 on PR #2380: flags could appear before OR
// after the positional command/block_id, and the original (raw-index)
// parser silently misbehaved depending on which side they landed on.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { checkSpawnerTier, logSrvVersion, parseArgs, renderLayout, renderWork } from "./muxspect.mjs";

describe("muxspect parseArgs", () => {
    it("'layout' with no tab id parses as a bare command", () => {
        expect(parseArgs(["layout"])).toEqual({ cmd: "layout", blockId: undefined, json: false, help: false });
    });

    it("'layout <tab_id>' puts the tab id in the generic positional", () => {
        // `layout` reuses muxspect's single positional slot for an optional
        // tab id rather than adding a parser special case.
        expect(parseArgs(["layout", "tab-1"])).toEqual({
            cmd: "layout",
            blockId: "tab-1",
            json: false,
            help: false,
        });
    });

    it("'layout --json' works with the flag on either side of the positional", () => {
        expect(parseArgs(["layout", "tab-1", "--json"]).json).toBe(true);
        expect(parseArgs(["--json", "layout", "tab-1"]).json).toBe(true);
        expect(parseArgs(["--json", "layout", "tab-1"]).blockId).toBe("tab-1");
    });

    it("no args defaults to 'list'", () => {
        expect(parseArgs([])).toEqual({ cmd: "list", blockId: undefined, json: false, help: false });
    });

    it("plain 'describe <id>'", () => {
        expect(parseArgs(["describe", "block-1"])).toEqual({
            cmd: "describe",
            blockId: "block-1",
            json: false,
            help: false,
        });
    });

    it("flag AFTER the command and block_id: 'describe block-1 --json'", () => {
        const r = parseArgs(["describe", "block-1", "--json"]);
        expect(r.cmd).toBe("describe");
        expect(r.blockId).toBe("block-1");
        expect(r.json).toBe(true);
    });

    it("flag BETWEEN the command and block_id: 'describe --json block-1' (reagent P2 on PR #2380)", () => {
        const r = parseArgs(["describe", "--json", "block-1"]);
        expect(r.cmd, "must still resolve to describe, not fall back to list").toBe("describe");
        expect(r.blockId, "must not pick up '--json' itself as the block_id").toBe("block-1");
        expect(r.json).toBe(true);
    });

    it("flag BEFORE the command: '--json describe block-1' (reagent P2 on PR #2380)", () => {
        const r = parseArgs(["--json", "describe", "block-1"]);
        expect(r.cmd, "must not fall back to list just because argv[0] is a flag").toBe("describe");
        expect(r.blockId).toBe("block-1");
        expect(r.json).toBe(true);
    });

    it("'watch <id>' parses the same way as describe", () => {
        const r = parseArgs(["watch", "block-1"]);
        expect(r.cmd).toBe("watch");
        expect(r.blockId).toBe("block-1");
    });

    it("'help' / '--help' / '-h' all set help regardless of position", () => {
        expect(parseArgs(["help"]).cmd).toBe("help");
        expect(parseArgs(["--help"]).help).toBe(true);
        expect(parseArgs(["list", "-h"]).help).toBe(true);
    });

    it("missing block_id for describe/watch is representable as undefined, not a flag string", () => {
        const r = parseArgs(["describe", "--json"]);
        expect(r.cmd).toBe("describe");
        expect(r.blockId, "no positional after 'describe' — must not be '--json'").toBeUndefined();
    });

    it("'dock <id>' parses like describe (no sub, blockId in positional[1])", () => {
        const r = parseArgs(["dock", "block-1"]);
        expect(r.cmd).toBe("dock");
        expect(r.sub).toBeUndefined();
        expect(r.blockId).toBe("block-1");
        expect(r.nodeId).toBeUndefined();
    });

    it("'dock clear <block_id> <node_id>' sets sub and both ids", () => {
        const r = parseArgs(["dock", "clear", "block-1", "node-1"]);
        expect(r.cmd).toBe("dock");
        expect(r.sub).toBe("clear");
        expect(r.blockId).toBe("block-1");
        expect(r.nodeId).toBe("node-1");
    });

    it("'dock clear' tolerates a flag anywhere, same discipline as describe", () => {
        const r = parseArgs(["dock", "--json", "clear", "block-1", "node-1"]);
        expect(r.sub, "must still resolve to clear, not fall back to plain dock").toBe("clear");
        expect(r.blockId).toBe("block-1");
        expect(r.nodeId).toBe("node-1");
        expect(r.json).toBe(true);
    });

    it("'dock' with only a block_id (no 'clear') never sets sub, even if positional[1] looks id-like", () => {
        const r = parseArgs(["dock", "not-the-word-clear"]);
        expect(r.sub).toBeUndefined();
        expect(r.blockId).toBe("not-the-word-clear");
    });

    it("'verify-sender <name>' parses like describe (name lands in blockId)", () => {
        const r = parseArgs(["verify-sender", "AgentA"]);
        expect(r.cmd).toBe("verify-sender");
        expect(r.blockId).toBe("AgentA");
    });

    // SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md Phase A —
    // both commands fall out of the existing generic parsing for free (no
    // 'sub' shape needed, unlike 'dock clear'); these tests just pin that.
    it("'conversations' (no arg) parses like plain 'list'", () => {
        const r = parseArgs(["conversations"]);
        expect(r.cmd).toBe("conversations");
        expect(r.blockId).toBeUndefined();
    });

    it("'conversation <agent>' parses like describe (agent name lands in blockId)", () => {
        const r = parseArgs(["conversation", "AgentA"]);
        expect(r.cmd).toBe("conversation");
        expect(r.blockId).toBe("AgentA");
    });

    it("'conversation <agent> --json' still resolves the agent name, flag anywhere", () => {
        const r = parseArgs(["conversation", "--json", "AgentA"]);
        expect(r.cmd).toBe("conversation");
        expect(r.blockId).toBe("AgentA");
        expect(r.json).toBe(true);
    });

    // Ext 4 (SPEC_MUXSPECT_CROSS_INSTANCE_FIND_2026_08_22.md) — 'find' parses
    // like 'conversation'/'describe': single positional lands in blockId, no
    // 'sub' shape needed. The UUID-vs-agent-name dispatch itself lives in
    // main() (query string routing, not argv parsing) — out of scope for
    // this pure parser, covered instead by manual/integration verification.
    it("'find <query>' parses like describe (query lands in blockId)", () => {
        const r = parseArgs(["find", "71a6b2ae-b651-43aa-aed4-6121f24fd713"]);
        expect(r.cmd).toBe("find");
        expect(r.blockId).toBe("71a6b2ae-b651-43aa-aed4-6121f24fd713");
    });

    it("'find <agent_name>' parses the same way for a non-UUID query", () => {
        const r = parseArgs(["find", "Korp"]);
        expect(r.cmd).toBe("find");
        expect(r.blockId).toBe("Korp");
    });

    it("'find <query> --json' still resolves the query, flag anywhere", () => {
        const r = parseArgs(["find", "--json", "Korp"]);
        expect(r.cmd).toBe("find");
        expect(r.blockId).toBe("Korp");
        expect(r.json).toBe(true);
    });
});

// SPEC_MUXSPECT_VERIFY_SENDER_2026_08_21.md tier 0 — checked before any
// network call, so it must work purely off process.env with no I/O.
describe("muxspect checkSpawnerTier", () => {
    it("matches when AGENTMUX_CHANNEL is a dev channel prefixed with the sender name", () => {
        const env = { AGENTMUX_CHANNEL: "dev-agenta-background-task-dashboard-intelligence-6c345e93dbc777e1" };
        const verdict = checkSpawnerTier("AgentA", env);
        expect(verdict).toEqual({
            name: "AgentA",
            status: "found",
            tier: "spawner",
            channel: env.AGENTMUX_CHANNEL,
        });
    });

    it("matches on AGENTMUX_RUNTIME_MODE when AGENTMUX_CHANNEL is absent", () => {
        const env = { AGENTMUX_RUNTIME_MODE: "dev:agenta-background-task-dashboard-intelligence" };
        const verdict = checkSpawnerTier("agenta", env);
        expect(verdict?.status).toBe("found");
        expect(verdict?.tier).toBe("spawner");
    });

    it("is case-insensitive", () => {
        const env = { AGENTMUX_CHANNEL: "dev-agenta-background-task-dashboard-intelligence-6c345e93dbc777e1" };
        expect(checkSpawnerTier("agenta", env)?.status).toBe("found");
    });

    it("returns null when the channel prefix names a DIFFERENT agent", () => {
        const env = { AGENTMUX_CHANNEL: "dev-someoneelse-other-feature-abc123" };
        expect(checkSpawnerTier("AgentA", env)).toBeNull();
    });

    it("returns null when neither env var is set (not a task-dev instance)", () => {
        expect(checkSpawnerTier("AgentA", {})).toBeNull();
    });

    it("returns null for a non-dev channel (e.g. plain 'stable')", () => {
        expect(checkSpawnerTier("AgentA", { AGENTMUX_CHANNEL: "stable" })).toBeNull();
    });
});

// Ext 5 (docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md):
// every response carries an x-agentmux-srv-version header (server/mod.rs's
// version_header middleware); logSrvVersion reads it and prints to stderr
// so a stale-build 404 is self-diagnosing instead of a bare, unexplained
// one. console.error is mocked (not asserted on message text — that's
// rendering detail) purely to keep test output clean; the return value is
// what callers (apiGet/apiPost's 404 version-hint) actually depend on.
describe("muxspect logSrvVersion", () => {
    let errSpy;

    beforeEach(() => {
        errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    });

    afterEach(() => {
        errSpy.mockRestore();
    });

    function fakeResponse(headerValue) {
        return { headers: { get: (name) => (name === "x-agentmux-srv-version" ? headerValue : null) } };
    }

    it("returns the version string when the header is present", () => {
        expect(logSrvVersion(fakeResponse("0.55.19"))).toBe("0.55.19");
    });

    it("logs to console.error (not stdout) when the header is present", () => {
        logSrvVersion(fakeResponse("0.55.19"));
        expect(errSpy).toHaveBeenCalledOnce();
    });

    it("returns undefined/falsy and logs nothing when the header is absent (older srv build)", () => {
        expect(logSrvVersion(fakeResponse(null))).toBeFalsy();
        expect(errSpy).not.toHaveBeenCalled();
    });
});

describe("muxspect renderLayout - whole-request failure", () => {
    let errSpy;
    let logSpy;
    beforeEach(() => {
        errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
        logSpy = vi.spyOn(console, "log").mockImplementation(() => {});
    });
    afterEach(() => {
        errSpy.mockRestore();
        logSpy.mockRestore();
    });

    // reagent P1 on PR #2856: a store-read failure returns 200 + {error} with
    // NO `layouts` key. Falling through to `data.layouts ?? []` printed
    // "no layouts found" and exited 0, reporting success for exactly the
    // on-disk failure this command exists to surface.
    it("prints the error to stderr instead of 'no layouts found'", () => {
        renderLayout({ error: "failed to list tabs: db locked" });
        expect(errSpy).toHaveBeenCalledWith(expect.stringContaining("failed to list tabs"));
        expect(logSpy).not.toHaveBeenCalledWith("no layouts found");
    });

    it("still says 'no layouts found' for a genuinely empty but SUCCESSFUL response", () => {
        renderLayout({ layouts: [] });
        expect(logSpy).toHaveBeenCalledWith("no layouts found");
        expect(errSpy).not.toHaveBeenCalled();
    });

    it("renders a per-tab error without swallowing the rest of the tabs", () => {
        renderLayout({
            layouts: [
                { tab_id: "t1", tab_name: "broken", error: "layoutstate unreadable" },
                {
                    tab_id: "t2",
                    tab_name: "ok",
                    healthy: true,
                    violations: [],
                    leaf_count: 1,
                    minimized_leaf_count: 0,
                    nodes: [],
                },
            ],
        });
        const printed = logSpy.mock.calls.map((c) => c.join(" ")).join(" | ");
        expect(printed).toContain("layoutstate unreadable");
        expect(printed).toContain("healthy");
    });
});

describe("muxspect renderWork", () => {
    let errSpy;
    let logSpy;
    beforeEach(() => {
        errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
        logSpy = vi.spyOn(console, "log").mockImplementation(() => {});
    });
    afterEach(() => {
        errSpy.mockRestore();
        logSpy.mockRestore();
    });

    const printed = () => logSpy.mock.calls.map((c) => c.join(" ")).join("\n");

    // REMOVED (reagent P2 on PR #2903): a test asserting renderWork printed a
    // top-level `data.error` to stderr. Unlike the layout endpoint, whose
    // handler deliberately returns 200-with-{error}, `/agentmux/work` returns
    // HTTP 500 — which apiGet turns into fail() + exit(1) before renderWork is
    // ever called. The branch was unreachable and this test verified a
    // response shape production never produces, which is worse than no test:
    // it made an uncovered failure mode look covered.

    it("says 'queue is empty' for a genuinely empty but SUCCESSFUL response", () => {
        renderWork({ items: [] }, "");
        expect(logSpy).toHaveBeenCalledWith("queue is empty");
        expect(errSpy).not.toHaveBeenCalled();
    });

    /// A filtered query that matches nothing is a DIFFERENT statement from an
    /// empty queue — conflating them is the same error class as the
    /// error-vs-empty one above, one level down.
    it("distinguishes 'no matches for this filter' from an empty queue", () => {
        renderWork({ items: [] }, "failed");
        expect(logSpy).toHaveBeenCalledWith("no failed items");
        expect(logSpy).not.toHaveBeenCalledWith("queue is empty");
    });

    it("shows state, holder, attempts, and the recorded result", () => {
        renderWork(
            {
                items: [
                    {
                        id: "w1",
                        title: "repro the thing",
                        state: "done",
                        claimed_by: "agentx",
                        attempts: 2,
                        max_attempts: 3,
                        result: "fixed in PR #123",
                    },
                ],
            },
            "",
        );
        const out = printed();
        expect(out).toContain("w1");
        expect(out).toContain("done");
        expect(out).toContain("held-by=agentx");
        expect(out).toContain("attempts=2/3");
        expect(out).toContain("fixed in PR #123");
    });

    /// The interesting pathology: a claimed row whose lease already lapsed.
    /// Nobody is working it, and nothing surfaces that until the next claim
    /// reaps it — so the reader must not have to compare epoch timestamps by
    /// eye to notice.
    it("flags a claimed item whose lease has already expired", () => {
        renderWork(
            {
                items: [
                    {
                        id: "w2",
                        title: "abandoned",
                        state: "claimed",
                        claimed_by: "ghost",
                        attempts: 1,
                        max_attempts: 3,
                        claim_expires: Date.now() - 60_000,
                    },
                ],
            },
            "",
        );
        const out = printed();
        expect(out).toContain("LEASE EXPIRED");
        expect(out).toContain("1 item(s) hold an EXPIRED lease");
    });

    it("does not flag a claimed item whose lease is still live", () => {
        renderWork(
            {
                items: [
                    {
                        id: "w3",
                        title: "in progress",
                        state: "claimed",
                        claimed_by: "worker",
                        attempts: 1,
                        max_attempts: 3,
                        claim_expires: Date.now() + 60_000,
                    },
                ],
            },
            "",
        );
        expect(printed()).not.toContain("LEASE EXPIRED");
    });

    /// Codex P2 on PR #2903. An expired lease does NOT always mean the item
    /// comes back: the reaper parks one whose attempts are already spent as
    /// `failed` instead of reopening it. Promising a comeback for those is
    /// exactly the false reassurance this command exists to prevent — and is
    /// the same over-promise WorkRelease made on #2902, repeated in the
    /// renderer.
    it("distinguishes an expired lease that will be reoffered from one that is doomed", () => {
        renderWork(
            {
                items: [
                    {
                        id: "back",
                        title: "will be reoffered",
                        state: "claimed",
                        claimed_by: "ghost",
                        attempts: 1,
                        max_attempts: 3,
                        claim_expires: Date.now() - 60_000,
                    },
                    {
                        id: "doomed",
                        title: "attempts spent",
                        state: "claimed",
                        claimed_by: "ghost",
                        attempts: 3,
                        max_attempts: 3,
                        claim_expires: Date.now() - 60_000,
                    },
                ],
            },
            "",
        );
        const out = printed();
        expect(out).toContain("ATTEMPTS SPENT");
        expect(out).toContain("NOT reoffered");
        // Both summary lines, each counting only its own case.
        expect(out).toContain("1 item(s) hold an EXPIRED lease — their claimant is gone");
        expect(out).toContain("1 item(s) hold an EXPIRED lease AND have spent every attempt");
    });

    /// Codex P2 on PR #2903: a full page must not be presented as the whole
    /// backlog. `work_queue_list` orders by `updated_at DESC`, so it is
    /// precisely the OLDEST open work and oldest expired claims — the things
    /// this command exists to surface — that fall off the end.
    it("warns that output may be truncated when a full page comes back", () => {
        const items = Array.from({ length: 3 }, (_, i) => ({
            id: `w${i}`,
            title: `item ${i}`,
            state: "open",
            attempts: 0,
            max_attempts: 3,
        }));
        renderWork({ items }, "", 3);
        expect(printed()).toContain("there may be MORE");
    });

    it("does not warn about truncation for a partial page", () => {
        const items = [{ id: "w0", title: "only one", state: "open", attempts: 0, max_attempts: 3 }];
        renderWork({ items }, "", 500);
        expect(printed()).not.toContain("there may be MORE");
    });
});
