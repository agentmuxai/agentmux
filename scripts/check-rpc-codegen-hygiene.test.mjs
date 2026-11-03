// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-rpc-codegen-hygiene.mjs against a made-up repo: an ambient global that duplicates a generated type, a typed
// handler that hand-serializes its response, and a generated import nothing uses each fail; clean code passes.

import { afterEach, describe, expect, it } from "vitest";
import { makeFixture } from "./lib/check-fixture.mjs";

let fx;
afterEach(() => fx?.remove());

function run(files) {
    fx = makeFixture(["check-rpc-codegen-hygiene.mjs"], files);
    return fx.run("check-rpc-codegen-hygiene.mjs");
}

describe("check-rpc-codegen-hygiene", () => {
    it("passes on an empty repo", () => {
        const r = run({});
        expect(r.status).toBe(0);
        expect(r.out).toContain("ok (3 checks)");
    });

    it("fails on an ambient global that duplicates a generated type", () => {
        const r = run({
            "frontend/types/rpc/DirEntry.ts": "export type DirEntry = {};\n",
            "frontend/types/custom.d.ts": "declare global {\n    type DirEntry = { name: string };\n}\n",
        });
        expect(r.status).toBe(1);
        expect(r.out).toContain("frontend/types/custom.d.ts:2  DirEntry");
    });

    it("allows the documented TokenCounts carve-out", () => {
        const r = run({
            "frontend/types/rpc/TokenCounts.ts": "export type TokenCounts = {};\n",
            "frontend/types/srv-types.d.ts": "declare global {\n    type TokenCounts = {};\n}\n",
        });
        expect(r.status).toBe(0);
    });

    it("fails on a typed handler that returns serde_json::to_value", () => {
        const r = run({
            "crates/srv/src/server/h.rs":
                "fn reg(engine: &mut E) {\n    engine.register_typed(\n        COMMAND_GETTHING,\n        |c| async move {\n            Ok(serde_json::to_value(x)?)\n        },\n    );\n}\n",
        });
        expect(r.status).toBe(1);
        expect(r.out).toContain("COMMAND_GETTHING");
    });

    it("fails on an inline json! response", () => {
        const r = run({
            "crates/srv/src/server/h.rs": "fn reg(engine: &mut E) {\n    engine.register_typed(\n        COMMAND_MOVE,\n        |c| async move {\n            Ok(serde_json::json!({ \"ok\": true }))\n        },\n    );\n}\n",
        });
        expect(r.status).toBe(1);
        expect(r.out).toContain("COMMAND_MOVE");
    });

    it("does not blame a typed handler for the next function's untyped one", () => {
        const r = run({
            "crates/srv/src/server/h.rs":
                "fn a(engine: &mut E) {\n    engine.register_typed(\n        COMMAND_A,\n        |c| async move { Ok(resp) },\n    );\n}\nfn b(engine: &mut E) {\n    engine.register_raw(\n        |c| async move {\n            Ok(serde_json::to_value(x)?)\n        },\n    );\n}\n",
        });
        expect(r.status).toBe(0);
    });

    it("fails on an unused generated import, and sees through an alias", () => {
        const r = run({
            "frontend/app/a.ts": 'import type { Used, Dead } from "@/types/rpc/x";\nconst u: Used = 1;\n',
            "frontend/app/b.ts": 'import type { Thing as ThingT } from "@/types/rpc/y";\nconst t: ThingT = 1;\n',
        });
        expect(r.status).toBe(1);
        expect(r.out).toContain("frontend/app/a.ts  Dead");
        expect(r.out).not.toContain("b.ts");
    });

    it("does not count a name that only appears in a comment or string", () => {
        const r = run({ "frontend/app/a.ts": 'import type { Ghost } from "@/types/rpc/x";\n// Ghost\nconst s = "Ghost";\n' });
        expect(r.status).toBe(1);
        expect(r.out).toContain("Ghost");
    });
});
