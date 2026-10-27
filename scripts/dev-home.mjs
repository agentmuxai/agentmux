#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Which data home `task dev` runs on. A dev build keeps its own database and logs under
// `<home>/dev/<branch>/<clone>`, but everything under `<home>/shared` (the agent list and
// transcripts, accounts, agent CLIs, the same-machine agent registry) and `<home>/agents` is
// shared by every instance that has the same home. So a dev build started with the default
// home shows, and acts on, the operator's real agents. This gives each dev build a home of its
// own unless the caller says otherwise:
//
//   node scripts/dev-home.mjs <clone root> <vite port>
//
// prints shell lines for `eval` on stdout and a one-line banner on stderr.
//
//   AGENTMUX_DEV_USE_REAL_HOME=1   use the real ~/.agentmux (the banner says REAL)
//   AGENTMUX_HOME_OVERRIDE=<dir>   use that folder (refused when it is the real home)
//   neither                        <clone root>/.dev-home/<vite port>, kept between runs

import { mkdirSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { homeProblem } from "./ui-screenshots/install-fake-claude.mjs";

export const USE_REAL_HOME = "AGENTMUX_DEV_USE_REAL_HOME";
export const HOME_DIR_NAME = ".dev-home";

/** The home a dev build runs on, and why: `{ mode: "isolated" | "override" | "real", home }`. Throws
 *  when `AGENTMUX_HOME_OVERRIDE` names the real home without `AGENTMUX_DEV_USE_REAL_HOME=1`. */
export function chooseHome({ env, cloneRoot, port, realHome = join(homedir(), ".agentmux") }) {
    if (env[USE_REAL_HOME] === "1") return { mode: "real", home: realHome };
    const given = env.AGENTMUX_HOME_OVERRIDE;
    if (given) {
        const problem = homeProblem(given, realHome);
        if (problem) throw new Error(`AGENTMUX_HOME_OVERRIDE: ${problem}. To run on your real data on purpose, set ${USE_REAL_HOME}=1.`);
        return { mode: "override", home: given };
    }
    return { mode: "isolated", home: join(cloneRoot, HOME_DIR_NAME, String(port)) };
}

/** The line a dev build logs and prints: where its data is, and whether that is the real one. */
export function banner({ mode, home }) {
    const where = forwardSlashes(home);
    if (mode === "real") return `[dev] home: ${where} (REAL: this build reads and changes your own agents, accounts and transcripts)`;
    return `[dev] home: ${where} (${mode === "override" ? "isolated, from AGENTMUX_HOME_OVERRIDE" : "isolated"})`;
}

const forwardSlashes = (p) => p.replace(/\\/g, "/");
const quote = (s) => `'${s.replace(/'/g, `'\\''`)}'`;

/** The shell lines that put `choice` in effect. The real home is the absence of an override. */
export function shellLines({ mode, home }) {
    const lines = [`export AGENTMUX_DEV_HOME_MODE=${mode}`, "unset AGENTMUX_DATA_HOME"];
    lines.push(mode === "real" ? "unset AGENTMUX_HOME_OVERRIDE" : `export AGENTMUX_HOME_OVERRIDE=${quote(forwardSlashes(home))}`);
    return lines;
}

function main(argv) {
    const [cloneRoot, port] = argv;
    if (!cloneRoot || !port) {
        process.stderr.write("usage: node scripts/dev-home.mjs <clone root> <vite port>\n");
        return 2;
    }
    let choice;
    try {
        choice = chooseHome({ env: process.env, cloneRoot: resolve(cloneRoot), port });
    } catch (e) {
        process.stderr.write(`❌ ${e.message}\n`);
        return 1;
    }
    if (choice.mode !== "real") mkdirSync(choice.home, { recursive: true });
    process.stderr.write(`${banner(choice)}\n`);
    process.stdout.write(`${shellLines(choice).join("\n")}\n`);
    return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) process.exit(main(process.argv.slice(2)));
