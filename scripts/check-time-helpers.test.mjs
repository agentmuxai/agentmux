// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-time-helpers.mjs against a made-up repo: a new hand-rolled epoch-time site fails, a grandfathered one
// passes, and a grandfathered file that no longer matches fails too, so the list only shrinks.

import { afterEach, describe, expect, it } from "vitest";
import { makeFixture } from "./lib/check-fixture.mjs";

const INLINE = "fn f() { let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap(); }\n";
const CLEAN = "fn f() { agentmux_common::time::now_ms(); }\n";

let fx;
afterEach(() => fx?.remove());

function repo(files, allow) {
    fx = makeFixture(["check-time-helpers.mjs"], { "scripts/check-time-helpers.allow": allow, ...files });
    return fx;
}

describe("check-time-helpers", () => {
    it("passes when every hand-rolled site is on the list", () => {
        const r = repo({ "crates/srv/src/a.rs": INLINE, "crates/srv/src/b.rs": CLEAN }, "# grandfathered\ncrates/srv/src/a.rs\n").run("check-time-helpers.mjs");
        expect(r.status).toBe(0);
        expect(r.out).toContain("ok (1 grandfathered files)");
    });

    it("fails on a new inline epoch computation", () => {
        const r = repo({ "crates/srv/src/new.rs": INLINE }, "").run("check-time-helpers.mjs");
        expect(r.status).toBe(1);
        expect(r.out).toContain("crates/srv/src/new.rs");
    });

    it("fails on a new fn now_ms of its own", () => {
        const r = repo({ "crates/mcp/src/x.rs": "fn now_ms() -> u64 { 0 }\n" }, "").run("check-time-helpers.mjs");
        expect(r.status).toBe(1);
        expect(r.out).toContain("crates/mcp/src/x.rs");
    });

    it("fails when a listed file no longer hand-rolls time, so the list only shrinks", () => {
        const r = repo({ "crates/srv/src/a.rs": CLEAN }, "crates/srv/src/a.rs\n").run("check-time-helpers.mjs");
        expect(r.status).toBe(1);
        expect(r.out).toContain("delete their lines");
    });

    it("skips migrations and tests/ folders", () => {
        const r = repo({ "crates/srv/src/migrations/m1.rs": INLINE, "crates/srv/src/tests/t.rs": INLINE }, "").run("check-time-helpers.mjs");
        expect(r.status).toBe(0);
    });

    it("--list prints the matching files and exits 0", () => {
        const r = repo({ "crates/srv/src/a.rs": INLINE }, "").run("check-time-helpers.mjs", ["--list"]);
        expect(r.status).toBe(0);
        expect(r.stdout.trim()).toBe("crates/srv/src/a.rs");
    });
});
