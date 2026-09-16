// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Node half of the DRY contract check described in
// docs/specs/SPEC_MUXSH_FULL_COLLECTION_2026_09_16.md §2.8: asserts every
// field muxsh's buildRequestBody() can ever emit for pane.open is listed in
// docs/specs/app-api-manifest.json's routes["pane.open"].requestFields — the
// same manifest a Rust test (agentmux-srv/src/backend/rpc_types/block.rs's
// app_api_manifest_contract_tests module) checks against the real
// CommandPaneOpenData struct. A field muxsh starts sending that isn't in the
// manifest (or that the manifest claims but the real Rust struct doesn't
// have) fails CI on whichever side drifted, instead of shipping as a silent
// mismatch discovered after merge — which is exactly how Phase 1's real bug
// (ReAgent, PR #3255, a field that should have been sent but wasn't) was
// actually found.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { buildRequestBody, parseArgs } from "./muxsh.mjs";

function loadManifest() {
    // agentmux-srv/src/backend/shellintegration/ -> repo root is four levels up.
    const here = dirname(fileURLToPath(import.meta.url));
    const manifestPath = join(here, "..", "..", "..", "..", "docs", "specs", "app-api-manifest.json");
    return JSON.parse(readFileSync(manifestPath, "utf8"));
}

describe("muxsh <-> app-api-manifest.json contract", () => {
    const manifest = loadManifest();
    const allowedFields = new Set(manifest.routes["pane.open"].requestFields);

    // Every combination of flags muxsh's own test suite exercises, plus a
    // couple of env-var combinations — a superset of what a real invocation
    // could ever produce, since buildRequestBody has no other input surface.
    const cases = [
        ["open bare, no env", parseArgs(["open", "/tmp/foo.md"]), {}],
        [
            "open with every flag, known block+tab",
            parseArgs(["open", "/tmp/foo.md", "--title", "t", "--split", "down", "--collapse-tree", "--floating", "--no-focus"]),
            { AGENTMUX_BLOCKID: "b-1", AGENTMUX_TABID: "t-1" },
        ],
        ["web bare, no env", parseArgs(["web", "https://example.com"]), {}],
        [
            "web with every flag, known block+tab",
            parseArgs(["web", "https://example.com", "--title", "t", "--split", "left", "--floating", "--no-focus"]),
            { AGENTMUX_BLOCKID: "b-1", AGENTMUX_TABID: "t-1" },
        ],
    ];

    it.each(cases)("%s: every emitted field is in the manifest's pane.open.requestFields", (_label, parsed, env) => {
        const body = buildRequestBody(parsed, env);
        const emitted = Object.keys(body);
        const unknown = emitted.filter((f) => !allowedFields.has(f));
        expect(unknown, `muxsh emitted field(s) not in the manifest: ${unknown.join(", ")}`).toEqual([]);
    });
});
