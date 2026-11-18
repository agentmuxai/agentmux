// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A release is published only when every package job succeeded: a published release can't take a file afterwards, so one that
// went out missing a package could not be completed. This fails if `publish` or `sign-release` of release-core.yml stops
// requiring a package job, if the job is not in their `needs`, or if a warning path for an incomplete package comes back.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const source = readFileSync(join(ROOT, ".github/workflows/release-core.yml"), "utf8").replace(/\r\n/g, "\n");

// The jobs that build or package something the release ships. A new one is added here, and the test then requires it.
const PACKAGE_JOBS = ["build-windows", "build-linux", "build-macos", "build-server-package"];

function jobBlock(name) {
    const start = source.search(new RegExp(`^  ${name}:\n`, "m"));
    expect(start, `job ${name} is in release-core.yml`).toBeGreaterThan(-1);
    const rest = source.slice(start + 1);
    const next = rest.search(/^  [a-z][a-z0-9-]*:\n/m);
    return next === -1 ? rest : rest.slice(0, next);
}

const condition = (block) => {
    const at = block.indexOf("    if: >-");
    expect(at, "the job has a multi-line if").toBeGreaterThan(-1);
    return block.slice(at, block.indexOf("    runs-on:", at));
};

describe("release-core.yml: what a release waits for", () => {
    for (const job of ["publish", "sign-release"]) {
        describe(job, () => {
            const block = jobBlock(job);
            const needs = block.match(/^    needs: \[([^\]]*)\]/m);
            const cond = condition(block);

            it("lists every package job in needs", () => {
                expect(needs, `${job} has a needs list`).not.toBeNull();
                const list = needs[1].split(",").map((n) => n.trim());
                for (const p of PACKAGE_JOBS) expect(list, `${job} needs ${p}`).toContain(p);
            });

            it("checks the result of every package job", () => {
                for (const p of PACKAGE_JOBS) expect(cond, `${job} checks needs.${p}.result`).toContain(`needs.${p}.result`);
            });

            it("requires the Headless server package to have succeeded, not skipped or failed", () => {
                expect(cond).toContain("needs.build-server-package.result == 'success'");
            });
        });
    }

    it("has no warning path for an incomplete package in publish", () => {
        const publish = jobBlock("publish");
        expect(publish).not.toMatch(/incomplete/i);
        expect(publish).not.toContain("PACKAGE_RESULT");
    });
});
