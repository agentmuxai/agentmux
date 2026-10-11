#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Installs the scripted stand-in for Claude Code (fake-claude/cli.mjs) as the
// Claude CLI of a capture instance — see
// docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md §10.
//
//   node scripts/ui-screenshots/install-fake-claude.mjs --home <AGENTMUX_HOME_OVERRIDE dir>
//        [--script <conversation.json>]
//
// srv looks for a provider's CLI in `<home>/shared/cli/<provider>/<pinned
// version>/` and uses it only when that folder is marked complete; this writes
// an npm-style shim there, the script beside it, and the marker. It refuses a
// --home that is the real ~/.agentmux: the stand-in must never replace the
// machine's own Claude Code.

import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const PROVIDERS_RS = join(here, "..", "..", "crates", "srv", "src", "backend", "providers.rs");
export const COMPLETE_MARKER = ".agentmux-install-complete";
const PACKAGE_DIR = "agentmux-fake-claude";

/** Claude's pinned version, read from srv's provider table (the `pinned_version`
 *  after `npm_package: "@anthropic-ai/claude-code"`). */
export function pinnedClaudeVersion(source = readFileSync(PROVIDERS_RS, "utf8")) {
    const at = source.indexOf('npm_package: "@anthropic-ai/claude-code"');
    const m = at >= 0 ? /pinned_version:\s*"([^"]+)"/.exec(source.slice(at)) : null;
    if (!m) throw new Error("couldn't find Claude's pinned_version in providers.rs");
    return m[1];
}

/** The npm-style Windows shim srv parses (crates/common/src/cli.rs): it runs
 *  `node <script>` for a line with `%dp0%\…\x.mjs" %*`. */
export function windowsShim() {
    return [
        "@ECHO off",
        "SETLOCAL",
        "SET dp0=%~dp0",
        'SET "_prog=node"',
        `"%_prog%" "%dp0%\\..\\${PACKAGE_DIR}\\cli.mjs" %*`,
        "",
    ].join("\r\n");
}

export function posixShim() {
    return ['#!/bin/sh', `exec node "$(dirname "$0")/../${PACKAGE_DIR}/cli.mjs" "$@"`, ""].join("\n");
}

/** Why `home` mustn't be used, or null: it's the machine's own data folder. */
export function homeProblem(home, realHome = join(homedir(), ".agentmux")) {
    const norm = (p) => resolve(p).replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
    return norm(home) === norm(realHome) ? "it is this machine's own ~/.agentmux" : null;
}

/** Installs the stand-in into `home`'s CLI folder; returns that folder. */
export function installFakeClaude(home, script = join(here, "fake-claude", "conversation.json")) {
    const problem = homeProblem(home);
    if (problem) throw new Error(`${home}: ${problem}; use a capture instance's own data folder`);
    if (!existsSync(join(home, "shared"))) throw new Error(`no shared folder under ${home}: start the capture instance once first`);
    const version = pinnedClaudeVersion();
    const dir = join(home, "shared", "cli", "claude", version);
    const pkg = join(dir, "node_modules", PACKAGE_DIR);
    const bin = join(dir, "node_modules", ".bin");
    mkdirSync(pkg, { recursive: true });
    mkdirSync(bin, { recursive: true });
    copyFileSync(join(here, "fake-claude", "cli.mjs"), join(pkg, "cli.mjs"));
    copyFileSync(script, join(pkg, "conversation.json"));
    writeFileSync(join(bin, "claude.cmd"), windowsShim());
    writeFileSync(join(bin, "claude"), posixShim(), { mode: 0o755 });
    writeFileSync(join(dir, COMPLETE_MARKER), version);
    return dir;
}

function main() {
    const argv = process.argv.slice(2);
    const get = (flag) => {
        const i = argv.indexOf(flag);
        return i >= 0 ? argv[i + 1] : undefined;
    };
    const home = get("--home");
    if (!home) {
        console.error("usage: install-fake-claude.mjs --home <AGENTMUX_HOME_OVERRIDE dir> [--script <conversation.json>]");
        process.exit(2);
    }
    try {
        console.log(`fake Claude Code: ${installFakeClaude(home, get("--script"))}`);
    } catch (err) {
        console.error(err.message);
        process.exit(2);
    }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main();
