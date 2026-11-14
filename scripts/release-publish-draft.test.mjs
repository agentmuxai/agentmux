// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The release is uploaded as a draft and published in one step, because a published release can't be changed
// (its files and its tag are fixed once it is out). This fails if the publish step creates the release
// published, publishes before the files are in, or touches a release that is already published.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const source = readFileSync(join(ROOT, ".github/workflows/release-core.yml"), "utf8").replace(/\r\n/g, "\n");
const start = source.indexOf("      - name: Create GitHub Release");
const step = source.slice(start, source.indexOf("\n      # ", start + 1));

describe("release-core.yml publish step", () => {
    it("is found", () => {
        expect(start).toBeGreaterThan(-1);
        expect(step).toContain("gh release create");
    });

    it("creates the release as a draft, on a tag that exists", () => {
        const create = step.slice(step.indexOf("gh release create"), step.indexOf("dist-flat/*", step.indexOf("gh release create")));
        expect(create).toContain("--draft");
        expect(create).toContain("--verify-tag");
    });

    it("publishes only after the files are in", () => {
        expect(step.indexOf("--draft=false")).toBeGreaterThan(step.indexOf("gh release create"));
        expect(step.indexOf("--draft=false")).toBeGreaterThan(step.indexOf("gh release upload"));
        expect(step.match(/--draft=false/g)).toHaveLength(1);
    });

    it("stops with a clear message on a release that is already published", () => {
        const published = step.indexOf("cannot be changed; cut a new version");
        expect(published).toBeGreaterThan(-1);
        expect(published).toBeLessThan(step.indexOf("gh release upload"));
        expect(step.slice(published, published + 80)).toContain("exit 1");
    });
});
