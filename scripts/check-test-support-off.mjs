#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-test-support-off.mjs: a shipped build never has the test-only feature.
//
// The `test-support` feature of agentmux-srv exists for test builds (the package turns it on for its own
// tests through a dev-dependency). A build that ships must resolve without it. Two checks:
//
//   node scripts/check-test-support-off.mjs             no build or packaging command asks for it:
//                                                       no `--all-features`, no `test-support`, in the
//                                                       workflows, Taskfile.yml, scripts, docker/ or .cargo/
//   node scripts/check-test-support-off.mjs --resolved  a normal (non-test) resolve of the workspace has it
//                                                       off for every package here (needs cargo)
//
// The compile-time half is in crates/srv/src/lib.rs: a release build with the feature on does not compile.
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./lib/is-main.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const FEATURE = "test-support";

/** Lines of `text` that ask for every feature or name the test-only one, with their numbers; comment lines skipped. */
export function commandProblems(text) {
    const out = [];
    text.split("\n").forEach((line, i) => {
        if (/^\s*(#|\/\/)/.test(line)) return;
        if (/--all-features\b/.test(line)) out.push({ line: i + 1, what: "--all-features" });
        else if (new RegExp(`(?<![\\w-])${FEATURE}(?![\\w-])`).test(line)) out.push({ line: i + 1, what: FEATURE });
    });
    return out;
}

const SCANNED = /\.(ya?ml|sh|bash|ps1|cmd|bat|mjs|js|py|toml|json)$|(^|\/)Dockerfile[^/]*$/;
const SKIPPED = /(^|\/)(check-test-support-off(\.test)?\.mjs|[^/]*\.test\.[mc]?[jt]s|node_modules|fixtures?)(\/|$)/;

/** The files that can carry a build or packaging command, as paths under `root`. */
export function commandFiles(root) {
    const out = [];
    const walk = (p) => {
        if (!existsSync(p)) return;
        const rel = relative(root, p).replaceAll("\\", "/");
        if (SKIPPED.test(rel)) return;
        if (statSync(p).isDirectory()) {
            for (const name of readdirSync(p)) walk(join(p, name));
        } else if (SCANNED.test(rel) || rel === "Taskfile.yml") {
            out.push(p);
        }
    };
    for (const top of [".github", "scripts", "docker", ".cargo", "Taskfile.yml"]) walk(join(root, top));
    return out;
}

/**
 * What a `cargo tree -f "{p} features=[{f}]" --prefix none` of the normal and build edges says about this
 * workspace: the packages (those with a local path) that have the feature on, and an error if agentmux-srv
 * is not in the output at all, so an empty answer cannot pass.
 */
export function resolvedProblems(tree) {
    const out = [];
    let sawSrv = false;
    for (const line of tree.split("\n")) {
        const m = /^(\S+) v\S+ \(([^)]+)\) features=\[([^\]]*)\]/.exec(line.trim());
        if (!m || m[2].includes("://")) continue;
        if (m[1] === "agentmux-srv") sawSrv = true;
        if (m[3].split(",").map((f) => f.trim()).includes(FEATURE)) out.push(`${m[1]} resolves with ${FEATURE} on in a normal build`);
    }
    if (!sawSrv) out.push("the resolve did not list agentmux-srv: the check read nothing");
    return [...new Set(out)];
}

function scanCommands() {
    const found = [];
    for (const file of commandFiles(ROOT)) {
        for (const { line, what } of commandProblems(readFileSync(file, "utf8"))) {
            found.push(`${relative(ROOT, file).replaceAll("\\", "/")}:${line}: ${what}`);
        }
    }
    if (found.length) {
        console.error(`check-test-support-off: a shipped build never has the test-only feature; these ask for it:\n  ${found.join("\n  ")}`);
        process.exit(1);
    }
    console.log("check-test-support-off: ok (no build or packaging command asks for the test-only feature)");
}

function checkResolved() {
    const run = spawnSync("cargo", ["tree", "--workspace", "-e", "normal,build", "--locked", "-f", "{p} features=[{f}]", "--prefix", "none"], {
        cwd: ROOT,
        encoding: "utf8",
        maxBuffer: 1 << 26,
    });
    if (run.status !== 0) {
        console.error(`check-test-support-off: cargo tree failed (${run.status ?? run.error}):\n${run.stderr ?? ""}`);
        process.exit(1);
    }
    const found = resolvedProblems(run.stdout);
    if (found.length) {
        console.error(`check-test-support-off: a shipped build never has the test-only feature:\n  ${found.join("\n  ")}`);
        process.exit(1);
    }
    console.log("check-test-support-off: ok (a normal build of every package here resolves without the test-only feature)");
}

if (isMain(import.meta.url)) {
    if (process.argv.includes("--resolved")) checkResolved();
    else scanCommands();
}
