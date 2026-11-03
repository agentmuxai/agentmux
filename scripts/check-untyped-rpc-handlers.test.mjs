// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-untyped-rpc-handlers.mjs against a made-up crates/srv: the numbers it holds (no untyped handler, no
// hand-built event, two marked relays, six marked typed helpers) are the baseline; anything off them fails.

import { afterEach, describe, expect, it } from "vitest";
import { makeFixture } from "./lib/check-fixture.mjs";

const GATE = "check-untyped-rpc-handlers.mjs";
const RELAY = "// untyped-relay: forwards an event built elsewhere\nlet e = MuxEvent { a };\n";
const HELPER = "// typed-helper: builds an event from a declared type\nlet e = MuxEvent { a };\n";

/** The baseline sources: 2 relays and 6 typed helpers. */
function baseline() {
    return { "crates/srv/src/relays.rs": RELAY.repeat(2), "crates/srv/src/helpers.rs": HELPER.repeat(6) };
}

let fx;
afterEach(() => fx?.remove());

function run(extra = {}, args = []) {
    fx = makeFixture([GATE], { ...baseline(), ...extra });
    return fx.run(GATE, args);
}

describe("check-untyped-rpc-handlers", () => {
    it("passes on the baseline", () => {
        const r = run();
        expect(r.status).toBe(0);
        expect(r.out).toContain("ok (0 untyped, none added)");
    });

    it("fails on a register_handler call", () => {
        const r = run({ "crates/srv/src/h.rs": "fn r(e: &mut E) { e.register_handler(\"x\", f); }\n" });
        expect(r.status).toBe(1);
        expect(r.out).toContain("1 register_handler calls, allowed 0");
    });

    it("ignores register_handler inside a test-only module, in a comment, and in tests.rs", () => {
        const r = run({
            "crates/srv/src/h.rs": "// e.register_handler(\"x\", f);\n#[cfg(test)]\nmod t {\n    fn r(e: &mut E) { e.register_handler(\"x\", f); }\n}\n",
            "crates/srv/src/tests.rs": "fn r(e: &mut E) { e.register_handler(\"x\", f); }\n",
        });
        expect(r.status).toBe(0);
    });

    it("still counts register_handler under not(test)", () => {
        const r = run({ "crates/srv/src/h.rs": "#[cfg(not(test))]\nmod t {\n    fn r(e: &mut E) { e.register_handler(\"x\", f); }\n}\n" });
        expect(r.status).toBe(1);
    });

    it("fails on a hand-built MuxEvent that carries no marker", () => {
        const r = run({ "crates/srv/src/e.rs": "fn p() { let e = MuxEvent { a }; }\n" });
        expect(r.status).toBe(1);
        expect(r.out).toContain("1 hand-built events, allowed 0");
    });

    it("fails on a hand-built eventrecv frame", () => {
        const r = run({ "crates/srv/src/e.rs": "fn p() { json!({ \"command\": \"eventrecv\" }); }\n" });
        expect(r.status).toBe(1);
        expect(r.out).toContain("hand-built events");
    });

    it("does not count a struct or impl of MuxEvent", () => {
        const r = run({ "crates/srv/src/e.rs": "pub struct MuxEvent {\n    a: u8,\n}\nimpl MuxEvent {\n}\n" });
        expect(r.status).toBe(0);
    });

    it("fails when a relay marker is added or removed", () => {
        const added = run({ "crates/srv/src/more.rs": RELAY });
        expect(added.status).toBe(1);
        expect(added.out).toContain("3 relay(s) marked untyped-relay, expected 2");
        fx.remove();
        fx = makeFixture([GATE], { "crates/srv/src/helpers.rs": HELPER.repeat(6), "crates/srv/src/relays.rs": RELAY });
        const removed = fx.run(GATE);
        expect(removed.status).toBe(1);
        expect(removed.out).toContain("1 relay(s) marked untyped-relay, expected 2");
    });

    it("fails when a typed-helper marker is added", () => {
        const r = run({ "crates/srv/src/more.rs": HELPER });
        expect(r.status).toBe(1);
        expect(r.out).toContain("7 site(s) marked typed-helper, expected 6");
    });

    it("--list prints each site and the counts", () => {
        const r = run({ "crates/srv/src/h.rs": "fn r(e: &mut E) { e.register_handler(\"x\", f); }\n" }, ["--list"]);
        expect(r.status).toBe(0);
        expect(r.stdout).toContain("crates/srv/src/h.rs:1");
        expect(r.stdout).toContain("1 untyped handler(s)");
        expect(r.stdout).toContain("2 relay(s)");
        expect(r.stdout).toContain("6 typed helper(s)");
    });
});
