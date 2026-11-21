// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-fixture.mjs: the environment a gate runs in inside a fixture.

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { makeFixture } from "./check-fixture.mjs";

const PRINT_BASE = 'process.stdout.write(process.env.GITHUB_BASE_REF ?? "unset");\n';

let fx;
let saved;
beforeEach(() => {
    saved = process.env.GITHUB_BASE_REF;
    process.env.GITHUB_BASE_REF = "some-stacked-branch";
});
afterEach(() => {
    fx?.remove();
    if (saved === undefined) delete process.env.GITHUB_BASE_REF;
    else process.env.GITHUB_BASE_REF = saved;
});

describe("makeFixture run", () => {
    it("does not pass the pull request's base branch from the environment to the gate", () => {
        fx = makeFixture([], {}).write("scripts/print-base.mjs", PRINT_BASE);
        expect(fx.run("print-base.mjs").stdout).toBe("unset");
    });

    it("passes a base branch the test sets itself", () => {
        fx = makeFixture([], {}).write("scripts/print-base.mjs", PRINT_BASE);
        expect(fx.run("print-base.mjs", [], { env: { GITHUB_BASE_REF: "main" } }).stdout).toBe("main");
    });
});
