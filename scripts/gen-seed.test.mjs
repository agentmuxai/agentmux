// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// agent-seed.json is what srv seeds; gen-seed.js is how it is meant to be
// edited. They drifted once (Antigravity, Pi's description and a seed memory
// were added to the JSON only), so a regeneration would have dropped them.
import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, it } from "vitest";

const script = resolve(dirname(fileURLToPath(import.meta.url)), "gen-seed.js");

it("agent-seed.json is what gen-seed.js generates", () => {
    expect(execFileSync(process.execPath, [script, "--check"], { encoding: "utf8" })).toContain("matches");
});
