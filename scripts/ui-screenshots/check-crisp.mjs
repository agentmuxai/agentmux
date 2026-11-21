#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Fails a screenshot that is not crisp, so blur cannot come back
// (docs/specs/SPEC_WEBSITE_DOCS_MEDIA_REFRESH_2026_11_20.md §5.3).
//
//   node scripts/ui-screenshots/check-crisp.mjs --run <run dir> [--run ...]
//       Each shot in the run's manifest.json: captured at scale 2 or more, and its pixels are its CSS size times
//       that scale; the PNG on disk has the size the manifest says.
//   node scripts/ui-screenshots/check-crisp.mjs --dir <folder> [--slot <css px>] [--exempt <regex>] [--list]
//       Every raster image under a folder (a docs clone's src/assets, a landing clone's public/screenshots):
//       at least twice as wide as the slot it is shown in (the docs' content column, 720 CSS px, unless --slot).
//
// Prints one line per failure (and, with --list, per file) and exits 1 when any file fails. SVG is not checked;
// icons, link-preview images and store badges are exempt (--exempt adds to that list).

import { existsSync, readFileSync, readdirSync } from "node:fs";
import { extname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DOCS_SLOT, readImageSize, shotProblems, slotProblem } from "./lib/crisp.mjs";

const RASTER = new Set([".png", ".webp", ".jpg", ".jpeg", ".gif"]);
/** Images whose size others define: icons, link-preview (og) images and store badges. */
export const DEFAULT_EXEMPT = /(^|\/)(favicon|apple-touch-icon|og-image|agentmux-logo|.*-badge)[^/]*$/i;

/** Problems of every shot in a run's manifest (an array of `{ where, problems }`, failures only). */
export function checkRun(dir) {
    const manifestPath = join(dir, "manifest.json");
    if (!existsSync(manifestPath)) throw new Error(`no manifest.json in ${dir}`);
    const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
    const failures = [];
    for (const shot of manifest.shots ?? []) {
        const problems = shotProblems(shot);
        const file = join(dir, shot.file ?? "");
        if (shot.file && existsSync(file)) {
            const size = readImageSize(readFileSync(file));
            if (size && (size.width !== shot.width || size.height !== shot.height)) {
                problems.push(`the file is ${size.width}x${size.height} px; the manifest says ${shot.width}x${shot.height}`);
            }
        }
        if (problems.length) failures.push({ where: relative(process.cwd(), file) || shot.id, problems });
    }
    return failures;
}

function* walk(dir) {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
        if (e.name === "node_modules" || e.name === ".git" || e.name === "dist") continue;
        const p = join(dir, e.name);
        if (e.isDirectory()) yield* walk(p);
        else yield p;
    }
}

/** Every raster under `root` against `slot`: `{ file, width, height, problem }`, `problem` null when it passes. */
export function checkDir(root, { slot = DOCS_SLOT, exempt = DEFAULT_EXEMPT } = {}) {
    const rows = [];
    for (const file of walk(root)) {
        if (!RASTER.has(extname(file).toLowerCase())) continue;
        const rel = relative(root, file).replaceAll("\\", "/");
        if (exempt.test(rel)) continue;
        const size = readImageSize(readFileSync(file));
        if (!size) {
            rows.push({ file: rel, width: 0, height: 0, problem: "its size can't be read" });
            continue;
        }
        rows.push({ file: rel, ...size, problem: slotProblem(size.width, slot) });
    }
    return rows;
}

function main() {
    const argv = process.argv.slice(2);
    const runs = [];
    const dirs = [];
    let slot = DOCS_SLOT;
    let exempt = DEFAULT_EXEMPT;
    let list = false;
    for (let i = 0; i < argv.length; i++) {
        if (argv[i] === "--run") runs.push(resolve(argv[++i]));
        else if (argv[i] === "--dir") dirs.push(resolve(argv[++i]));
        else if (argv[i] === "--slot") slot = Number(argv[++i]);
        else if (argv[i] === "--exempt") exempt = new RegExp(`${DEFAULT_EXEMPT.source}|${argv[++i]}`, "i");
        else if (argv[i] === "--list") list = true;
        else {
            console.error(`unknown argument ${argv[i]}`);
            process.exit(2);
        }
    }
    if ((runs.length === 0 && dirs.length === 0) || !(slot > 0)) {
        console.error("usage: check-crisp.mjs --run <run dir> [--run ...] | --dir <folder> [--slot <css px>] [--exempt <regex>] [--list]");
        process.exit(2);
    }
    let failed = 0;
    try {
        for (const dir of runs) {
            const failures = checkRun(dir);
            for (const f of failures) console.log(`FAIL ${f.where}: ${f.problems.join("; ")}`);
            failed += failures.length;
        }
        for (const dir of dirs) {
            const rows = checkDir(dir, { slot, exempt });
            for (const r of rows) {
                if (r.problem) console.log(`FAIL ${r.file}: ${r.problem}`);
                else if (list) console.log(`ok   ${r.file}: ${r.width}x${r.height}`);
            }
            const bad = rows.filter((r) => r.problem).length;
            console.log(`${dir}: ${rows.length - bad} of ${rows.length} images are crisp for a ${slot} px slot`);
            failed += bad;
        }
    } catch (err) {
        console.error(err.message);
        process.exit(2);
    }
    if (failed) process.exitCode = 1;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main();
