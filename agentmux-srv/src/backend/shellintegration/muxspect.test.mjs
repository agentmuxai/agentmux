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
import { parseArgs } from "./muxspect.mjs";

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
});
