// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// resolve-runtime-tag.sh: the CEF runtime tag the three build workflows fetch, from the committed pin or an explicit tag.

import { spawnSync } from "node:child_process";
import { cpSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { HAS_BASH } from "../lib/check-fixture.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const d = HAS_BASH ? describe : describe.skip;

let dir;
afterEach(() => dir && rmSync(dir, { recursive: true, force: true }));

/** The script run next to a pins file holding `pins`. */
function run(pins, ...args) {
    dir = mkdtempSync(join(tmpdir(), "resolve-tag-"));
    cpSync(join(HERE, "resolve-runtime-tag.sh"), join(dir, "resolve-runtime-tag.sh"));
    writeFileSync(join(dir, "cef-runtime-pins.sh"), pins);
    const r = spawnSync("bash", [join(dir, "resolve-runtime-tag.sh"), ...args], { encoding: "utf8" });
    return { status: r.status, stdout: r.stdout, stderr: r.stderr };
}

const PINS = 'CEF_WINDOWS_RUNTIME_TAG="win-r1"\nCEF_LINUX_RUNTIME_TAG="lin-r1"\nCEF_MACOS_RUNTIME_TAG="mac-r1"\n';

d("resolve-runtime-tag.sh", () => {
    it.each([
        ["linux", "lin-r1"],
        ["macos", "mac-r1"],
        ["windows", "win-r1"],
    ])("reads the %s pin when no tag is given", (platform, want) => {
        const r = run(PINS, platform);
        expect(r.status).toBe(0);
        expect(r.stdout).toBe(`${want}\n`);
    });

    it("treats an empty tag argument as no tag", () => {
        expect(run(PINS, "linux", "").stdout).toBe("lin-r1\n");
    });

    it("lets an explicit tag win over the pin", () => {
        const r = run(PINS, "windows", "cef-windows-custom");
        expect(r.status).toBe(0);
        expect(r.stdout).toBe("cef-windows-custom\n");
    });

    it("does not need a pin when a tag is given", () => {
        expect(run("", "linux", "t1").stdout).toBe("t1\n");
    });

    it("fails when the pins file has no tag for the platform", () => {
        const r = run('CEF_WINDOWS_RUNTIME_TAG="win-r1"\n', "linux");
        expect(r.status).toBe(1);
        expect(r.stderr).toContain("no linux CEF runtime tag");
        expect(r.stdout).toBe("");
    });

    it("fails on a pin that is the word null", () => {
        const r = run('CEF_MACOS_RUNTIME_TAG="null"\n', "macos");
        expect(r.status).toBe(1);
        expect(r.stderr).toContain("no macos CEF runtime tag");
    });

    it("prints its usage and exits 2 for a platform it does not know", () => {
        const r = run(PINS, "plan9");
        expect(r.status).toBe(2);
        expect(r.stderr).toContain("usage:");
    });

    it("reads the repository's own pins file for every platform", () => {
        for (const platform of ["linux", "macos", "windows"]) {
            const r = spawnSync("bash", [join(HERE, "resolve-runtime-tag.sh"), platform], { encoding: "utf8" });
            expect(r.status, platform).toBe(0);
            expect(r.stdout.trim(), platform).toMatch(/^cef-/);
        }
    });
});
