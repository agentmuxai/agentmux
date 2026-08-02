// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for muxspect.mjs's argument parsing — pure, no network, no
// running instance needed. Runnable standalone:
//   node --test agentmux-srv/src/backend/shellintegration/muxspect.test.mjs
//
// Pins the fix for reagent P2 on PR #2380: flags could appear before OR
// after the positional command/block_id, and the original (raw-index)
// parser silently misbehaved depending on which side they landed on.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseArgs } from "./muxspect.mjs";

test("no args defaults to 'list'", () => {
    assert.deepEqual(parseArgs([]), { cmd: "list", blockId: undefined, json: false, help: false });
});

test("plain 'describe <id>'", () => {
    assert.deepEqual(parseArgs(["describe", "block-1"]), {
        cmd: "describe",
        blockId: "block-1",
        json: false,
        help: false,
    });
});

test("flag AFTER the command and block_id: 'describe block-1 --json'", () => {
    const r = parseArgs(["describe", "block-1", "--json"]);
    assert.equal(r.cmd, "describe");
    assert.equal(r.blockId, "block-1");
    assert.equal(r.json, true);
});

test("flag BETWEEN the command and block_id: 'describe --json block-1' (reagent P2 on PR #2380)", () => {
    const r = parseArgs(["describe", "--json", "block-1"]);
    assert.equal(r.cmd, "describe", "must still resolve to describe, not fall back to list");
    assert.equal(r.blockId, "block-1", "must not pick up '--json' itself as the block_id");
    assert.equal(r.json, true);
});

test("flag BEFORE the command: '--json describe block-1' (reagent P2 on PR #2380)", () => {
    const r = parseArgs(["--json", "describe", "block-1"]);
    assert.equal(r.cmd, "describe", "must not fall back to list just because argv[0] is a flag");
    assert.equal(r.blockId, "block-1");
    assert.equal(r.json, true);
});

test("'watch <id>' parses the same way as describe", () => {
    const r = parseArgs(["watch", "block-1"]);
    assert.equal(r.cmd, "watch");
    assert.equal(r.blockId, "block-1");
});

test("'help' / '--help' / '-h' all set help regardless of position", () => {
    assert.equal(parseArgs(["help"]).cmd, "help");
    assert.equal(parseArgs(["--help"]).help, true);
    assert.equal(parseArgs(["list", "-h"]).help, true);
});

test("missing block_id for describe/watch is representable as undefined, not a flag string", () => {
    const r = parseArgs(["describe", "--json"]);
    assert.equal(r.cmd, "describe");
    assert.equal(r.blockId, undefined, "no positional after 'describe' — must not be '--json'");
});
