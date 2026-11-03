// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-workdir-writes.mjs against a made-up repo: a raw filesystem write in a launch-config writer fails; one
// inside #[cfg(test)], one with a `// workdir-fs:` reason, and a write through Workdir pass.

import { afterEach, describe, expect, it } from "vitest";
import { makeFixture } from "./lib/check-fixture.mjs";

const AGENT_CONFIG = "crates/srv/src/backend/agent_config.rs";
const AGENT_OPEN = "crates/srv/src/server/app_api/agent_open.rs";
const EDITOR = "crates/srv/src/server/editor_handlers.rs";

let fx;
afterEach(() => fx?.remove());

function repo(overrides) {
    fx = makeFixture(["check-workdir-writes.mjs"], {
        [AGENT_CONFIG]: "fn a(w: &Workdir) { w.write(\"x\", b\"\"); }\n",
        [AGENT_OPEN]: "fn b() {}\n",
        [EDITOR]: "fn c() {}\n",
        ...overrides,
    });
    return fx.run("check-workdir-writes.mjs");
}

describe("check-workdir-writes", () => {
    it("passes when writes go through Workdir", () => {
        const r = repo({});
        expect(r.status).toBe(0);
        expect(r.out).toContain("check-workdir-writes: ok");
    });

    it("fails on a raw fs::write", () => {
        const r = repo({ [AGENT_CONFIG]: "fn a() {\n    std::fs::write(p, b\"\").unwrap();\n}\n" });
        expect(r.status).toBe(1);
        expect(r.out).toContain(`${AGENT_CONFIG}:2`);
    });

    it("fails on File::create and OpenOptions", () => {
        expect(repo({ [AGENT_OPEN]: "fn b() { let f = File::create(p); }\n" }).status).toBe(1);
        fx.remove();
        expect(repo({ [AGENT_OPEN]: "fn b() { OpenOptions::new(); }\n" }).status).toBe(1);
    });

    it("lets a line with a workdir-fs reason stay raw", () => {
        const r = repo({ [AGENT_CONFIG]: "fn a() { fs::create_dir_all(p); } // workdir-fs: creates the workdir itself\n" });
        expect(r.status).toBe(0);
    });

    it("ignores raw writes inside a #[cfg(test)] module", () => {
        const r = repo({ [AGENT_CONFIG]: "fn a() {}\n#[cfg(test)]\nmod tests {\n    fn t() { std::fs::write(p, b\"\"); }\n}\n" });
        expect(r.status).toBe(0);
    });

    it("checks the editor handler only inside its registration region", () => {
        const outside = repo({ [EDITOR]: "fn save() { std::fs::write(p, b\"\"); }\n" });
        expect(outside.status).toBe(0);
        fx.remove();
        const inside = repo({
            [EDITOR]: "engine.register_typed(\n    COMMAND_WRITE_AGENT_CONFIG,\n    |x| { std::fs::write(p, b\"\"); },\n);\nengine.register_typed(\n",
        });
        expect(inside.status).toBe(1);
    });
});
