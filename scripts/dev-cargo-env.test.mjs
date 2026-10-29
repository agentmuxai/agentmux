// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Guards the wiring of Taskfile.yml's `DEV_CARGO_ENV`: the cargo settings (no LTO, 16 codegen units, opt-level 1,
// incremental) that `task dev` and `task dev:standalone` build with, so a dev build on a shared host does not link
// the srv and the CEF host with full LTO. Package and release builds must NOT pass it: they keep Cargo.toml's
// release profile, and a release that silently lost LTO would ship bigger and slower with nothing to say so.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const taskfile = readFileSync(join(repoRoot, "Taskfile.yml"), "utf8");

/** The text of one top-level task, from its `    name:` line to the next task at the same indent. */
function taskBody(name) {
    const lines = taskfile.split(/\r?\n/);
    const start = lines.findIndex((l) => l === `    ${name}:`);
    if (start < 0) throw new Error(`no task ${name} in Taskfile.yml`);
    let end = lines.length;
    for (let i = start + 1; i < lines.length; i++) {
        if (/^ {4}[A-Za-z0-9:_-]+:\s*$/.test(lines[i])) {
            end = i;
            break;
        }
    }
    return lines.slice(start, end).join("\n");
}

const devOnly = ["dev", "dev:standalone", "dev:serve", "dev:standalone:serve"];

describe("DEV_CARGO_ENV: only the dev entry points build without LTO", () => {
    it("is defined as the four CARGO_PROFILE_RELEASE_* overrides, with LTO off", () => {
        const def = taskfile.split(/\r?\n/).find((l) => l.startsWith("    DEV_CARGO_ENV:"));
        expect(def).toBeTruthy();
        expect(def).toContain("CARGO_PROFILE_RELEASE_LTO=off");
        expect(def).toContain("CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16");
        expect(def).toContain("CARGO_PROFILE_RELEASE_OPT_LEVEL=1");
        expect(def).toContain("CARGO_PROFILE_RELEASE_INCREMENTAL=true");
    });

    it("is passed by every dev entry point and by nothing else", () => {
        const users = [...taskfile.matchAll(/^ {4}([A-Za-z0-9:_-]+):\s*$/gm)]
            .map((m) => m[1])
            .filter((name) => taskBody(name).includes("{{.DEV_CARGO_ENV}}"));
        expect(users.sort()).toEqual([...devOnly].sort());
    });

    it("the cargo release builds read it from the task's own CARGO_ENV, which is empty unless a caller passed it", () => {
        for (const name of ["build:backend:rust:windows", "build:backend:rust:linux", "build:backend:rust:darwin", "build:host:windows", "build:host:linux", "build:host:darwin"]) {
            const cargoLines = taskBody(name)
                .split(/\r?\n/)
                .filter((l) => /cargo build --release/.test(l) && !/^\s*#/.test(l));
            expect(cargoLines.length, name).toBeGreaterThan(0);
            for (const l of cargoLines) expect(l, `${name}: ${l.trim()}`).toContain("{{.CARGO_ENV}}");
        }
    });
});
