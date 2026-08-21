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

import { describe, expect, it } from "vitest";
import { checkSpawnerTier, parseArgs } from "./muxspect.mjs";

describe("muxspect parseArgs", () => {
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
            trust: "spawner-verified",
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
