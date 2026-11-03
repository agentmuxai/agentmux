// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-fixture.mjs: run a scripts/check-*.mjs or check-*.sh gate against a small made-up repo.
//
// These gates find the repo from where they sit (`new URL("..", import.meta.url)`, `cd "$(dirname "$0")/.."`)
// or from the current directory, so copying one into `<tmp>/scripts/` and writing the files it reads under
// `<tmp>` points it at the fixture, with no change to the gate itself.

import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPTS = dirname(dirname(fileURLToPath(import.meta.url)));

// Every fixture not yet removed. A test that makes several (or fails before its `afterEach`) leaves some behind; they go when the worker exits.
const LIVE = new Set();
process.on("exit", () => {
    for (const root of LIVE) rmSync(root, { recursive: true, force: true });
});

/**
 * A temp repo holding the named gates (and `scripts/lib`, which several import) plus `files`
 * ({ "relative/path": "text" }). Call `remove()` when done.
 */
export function makeFixture(gates, files = {}) {
    const root = mkdtempSync(join(tmpdir(), "check-fixture-"));
    LIVE.add(root);
    mkdirSync(join(root, "scripts"), { recursive: true });
    cpSync(join(SCRIPTS, "lib"), join(root, "scripts", "lib"), { recursive: true });
    for (const gate of gates) cpSync(join(SCRIPTS, gate), join(root, "scripts", gate));
    const fixture = {
        root,
        write(rel, text) {
            const p = join(root, rel);
            mkdirSync(dirname(p), { recursive: true });
            writeFileSync(p, text);
            return fixture;
        },
        /** Run `scripts/<gate>` in the fixture: `.mjs` with node, `.sh` with bash. */
        run(gate, args = [], opts = {}) {
            const [cmd, first] = gate.endsWith(".sh") ? [BASH, [join(root, "scripts", gate)]] : [process.execPath, [join(root, "scripts", gate)]];
            const r = spawnSync(cmd, [...first, ...args], { cwd: root, encoding: "utf8", env: { ...process.env, ...opts.env } });
            return { status: r.status, out: `${r.stdout}${r.stderr}`, stdout: r.stdout, stderr: r.stderr };
        },
        /** `git init` and track everything written so far, for gates that read `git ls-files`. */
        git() {
            const env = { ...process.env, GIT_AUTHOR_NAME: "t", GIT_AUTHOR_EMAIL: "t@t", GIT_COMMITTER_NAME: "t", GIT_COMMITTER_EMAIL: "t@t" };
            for (const args of [["init", "-q"], ["add", "-A"], ["commit", "-q", "-m", "fixture"]]) spawnSync("git", args, { cwd: root, env });
            return fixture;
        },
        /** Run `git <args>` in the fixture (after `git()`), for gates that compare against a merge-base. */
        gitRun(...args) {
            const env = { ...process.env, GIT_AUTHOR_NAME: "t", GIT_AUTHOR_EMAIL: "t@t", GIT_COMMITTER_NAME: "t", GIT_COMMITTER_EMAIL: "t@t" };
            const r = spawnSync("git", args, { cwd: root, env, encoding: "utf8" });
            if (r.status !== 0) throw new Error(`git ${args.join(" ")}: ${r.stderr}`);
            return r.stdout;
        },
        remove() {
            LIVE.delete(root);
            if (existsSync(root)) rmSync(root, { recursive: true, force: true });
        },
    };
    for (const [rel, text] of Object.entries(files)) fixture.write(rel, text);
    return fixture;
}

/** Whether `bash` runs here; the shell gates' tests skip when it does not. */
export const HAS_BASH = spawnSync("bash", ["-c", "exit 0"]).status === 0;

// Absolute, so a test that narrows `PATH` to a few fakes still finds the shell. (Windows keeps the bare name: its shell reports a path Node cannot spawn.)
const BASH = HAS_BASH && process.platform !== "win32" ? spawnSync("bash", ["-c", "command -v bash"], { encoding: "utf8" }).stdout.trim() || "bash" : "bash";
