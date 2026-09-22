// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Guards the wiring between Taskfile.yml's `release*` tasks and
// scripts/release.sh, which accepts `--dry-run`.
//
// WHY this exists: `release:patch` and `release:minor` were pinned to
// `bash scripts/release.sh --as patch` with no `{{.CLI_ARGS}}`, so
// `task release:patch -- --dry-run` SILENTLY DROPPED the flag and performed a
// real release — bumping every version file, deleting all pending changesets
// and staging the lot. Task does not warn about unconsumed CLI_ARGS, and
// release.sh never saw an argument to reject, so the only signal was the
// mutation itself. `task release -- --dry-run` worked, which made the failure
// look like a release.sh bug rather than a Taskfile one.
//
// A dropped flag is worse than a rejected one: it reads as honoured.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const taskfile = readFileSync(join(repoRoot, "Taskfile.yml"), "utf8");
const releaseSh = readFileSync(join(repoRoot, "scripts", "release.sh"), "utf8");

/**
 * Return the `cmds:` lines for a top-level task, by name.
 *
 * Deliberately a small text scan rather than a YAML parse: adding a YAML
 * dependency to guard four lines is not worth it, and the shape here is fixed
 * (top-level tasks are indented four spaces in this file).
 */
function cmdsFor(taskName) {
    const lines = taskfile.split(/\r?\n/);
    const start = lines.findIndex((l) => l.trim() === `${taskName}:`);
    if (start === -1) throw new Error(`task '${taskName}' not found in Taskfile.yml`);
    const indent = lines[start].length - lines[start].trimStart().length;

    const cmds = [];
    let inCmds = false;
    for (const line of lines.slice(start + 1)) {
        if (line.trim() === "") continue;
        const thisIndent = line.length - line.trimStart().length;
        // Dedent to the task's own level or beyond ends this task's block.
        if (thisIndent <= indent) break;
        if (line.trim() === "cmds:") {
            inCmds = true;
            continue;
        }
        if (inCmds && line.trim().startsWith("-")) cmds.push(line.trim().slice(1).trim());
    }
    return cmds;
}

const RELEASE_TASKS = ["release", "release:patch", "release:minor"];

describe("release.sh", () => {
    it("accepts --dry-run (the premise of the tests below)", () => {
        // If this ever stops being true, the forwarding assertions are
        // meaningless and should be deleted rather than left passing.
        expect(releaseSh).toMatch(/^\s*--dry-run\)/m);
    });
});

describe("Taskfile release tasks", () => {
    it.each(RELEASE_TASKS)("%s invokes scripts/release.sh", (task) => {
        const cmds = cmdsFor(task);
        expect(cmds.some((c) => c.includes("scripts/release.sh"))).toBe(true);
    });

    // The regression itself.
    it.each(RELEASE_TASKS)("%s forwards {{.CLI_ARGS}} so --dry-run survives", (task) => {
        const cmd = cmdsFor(task).find((c) => c.includes("scripts/release.sh"));
        expect(cmd).toContain("{{.CLI_ARGS}}");
    });

    it("keeps the forced type ahead of CLI_ARGS so the caller can still override", () => {
        // release.sh's parser takes the LAST --as it sees. Putting the task's
        // own `--as <type>` first means `task release:patch -- --as minor`
        // resolves to minor rather than silently ignoring the caller.
        for (const [task, type] of [
            ["release:patch", "patch"],
            ["release:minor", "minor"],
        ]) {
            const cmd = cmdsFor(task).find((c) => c.includes("scripts/release.sh"));
            expect(cmd.indexOf(`--as ${type}`)).toBeLessThan(cmd.indexOf("{{.CLI_ARGS}}"));
        }
    });
});
