#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Copies a run's screenshots and recordings over the ones agentmux-docs
// already uses — see docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md
// §13. The docs name each image after its shot (`<id>.png`, or
// `<id>-<size>.png` for a shot taken at several sizes) or clip
// (`<id>.gif`), so a file found under the docs' src/assets/ by that name is
// replaced. Nothing is added: an image the docs don't use yet is listed, to be
// placed by hand. A screenshot is copied only when its shot marks it
// publishable (`containsWorkspaceData` false, or "review": a person checks it);
// one marked true, or not marked at all, never is.
//
//   node scripts/ui-screenshots/publish.mjs --docs <agentmux-docs clone> --from <run dir> [--from ...] [--dry-run]
//
// Review the docs clone's `git diff` (and the images) before committing.

import { copyFileSync, existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { basename, extname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** The images a run directory holds: from its manifest.json, each with the
 *  name the docs use for it. */
export function runImages(manifest, dir) {
    const out = [];
    for (const s of manifest.shots ?? []) {
        out.push({
            source: join(dir, s.file),
            name: `${s.size ? `${s.id}-${s.size}` : s.id}${extname(s.file)}`,
            id: s.id,
            // Publishable only when the shot says so: false (shows no machine
            // data) or "review" (a person checks it). A shot that doesn't say
            // (the first run's manual suite) isn't copied (Codex on #4673).
            machineData: !(s.containsWorkspaceData === false || s.containsWorkspaceData === "review"),
        });
    }
    for (const r of manifest.recordings ?? []) {
        for (const f of r.files ?? []) out.push({ source: join(dir, f), name: basename(f), id: r.id, machineData: false });
    }
    return out;
}

/** Every file under `root`, by its name (a name found twice maps to all of them). */
export function filesByName(root) {
    const byName = new Map();
    const walk = (d) => {
        for (const e of readdirSync(d, { withFileTypes: true })) {
            const p = join(d, e.name);
            if (e.isDirectory()) walk(p);
            else byName.set(e.name, [...(byName.get(e.name) ?? []), p]);
        }
    };
    if (existsSync(root)) walk(root);
    return byName;
}

/** What publishing would do: `replace` (a docs file of that name exists),
 *  `unused` (the docs have none), `ambiguous` (several do), `skipped` (shows
 *  this machine's data, or the run has no such file). */
export function plan(images, docsByName) {
    // Two runs with an image of the same name (the manual suite shares ids
    // with the agent and memory suites): neither is copied (Codex on #4673).
    const runsWith = new Map();
    for (const img of images) runsWith.set(img.name, (runsWith.get(img.name) ?? 0) + 1);
    return images.map((img) => {
        if (runsWith.get(img.name) > 1) return { ...img, action: "ambiguous", targets: [], why: "more than one run has an image of this name" };
        if (img.machineData) return { ...img, action: "skipped", why: "shows this machine's data, or isn't marked publishable" };
        if (!existsSync(img.source)) return { ...img, action: "skipped", why: "not in the run" };
        const targets = docsByName.get(img.name) ?? [];
        if (targets.length === 0) return { ...img, action: "unused" };
        if (targets.length > 1) return { ...img, action: "ambiguous", targets };
        return { ...img, action: "replace", target: targets[0] };
    });
}

function sameBytes(a, b) {
    if (statSync(a).size !== statSync(b).size) return false;
    return readFileSync(a).equals(readFileSync(b));
}

/** Publishes the runs in `from` into the docs clone at `docs`; returns the plan. */
export function publish({ docs, from, dryRun = false, log = console.log }) {
    const assets = join(docs, "src", "assets");
    if (!existsSync(assets)) throw new Error(`${docs} doesn't look like an agentmux-docs clone (no src/assets)`);
    const images = [];
    for (const dir of from) {
        const m = join(dir, "manifest.json");
        if (!existsSync(m)) throw new Error(`no manifest.json in ${dir}`);
        images.push(...runImages(JSON.parse(readFileSync(m, "utf8")), dir));
    }
    const steps = plan(images, filesByName(assets));
    let changed = 0;
    for (const s of steps) {
        if (s.action === "replace") {
            const differs = !sameBytes(s.source, s.target);
            if (differs && !dryRun) copyFileSync(s.source, s.target);
            if (differs) changed++;
            log(`${differs ? (dryRun ? "would update" : "updated") : "unchanged"}  ${relative(docs, s.target)}`);
        } else if (s.action === "unused") log(`not in the docs  ${s.name}`);
        else if (s.action === "ambiguous") log(`AMBIGUOUS  ${s.name}: ${s.why ?? s.targets.map((t) => relative(docs, t)).join(", ")}`);
        else log(`skipped  ${s.name} (${s.why})`);
    }
    const count = (a) => steps.filter((s) => s.action === a).length;
    log(`\n${changed} ${dryRun ? "would change" : "changed"}, ${count("replace") - changed} unchanged, ${count("unused")} not in the docs, ${count("skipped")} skipped, ${count("ambiguous")} ambiguous`);
    return steps;
}

function main() {
    const argv = process.argv.slice(2);
    const from = [];
    let docs = null;
    let dryRun = false;
    for (let i = 0; i < argv.length; i++) {
        if (argv[i] === "--from") from.push(resolve(argv[++i]));
        else if (argv[i] === "--docs") docs = resolve(argv[++i]);
        else if (argv[i] === "--dry-run") dryRun = true;
    }
    if (!docs || from.length === 0) {
        console.error("usage: publish.mjs --docs <agentmux-docs clone> --from <run dir> [--from ...] [--dry-run]");
        process.exit(2);
    }
    try {
        const steps = publish({ docs, from, dryRun });
        if (steps.some((s) => s.action === "ambiguous")) process.exitCode = 1;
    } catch (err) {
        console.error(err.message);
        process.exit(2);
    }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main();
