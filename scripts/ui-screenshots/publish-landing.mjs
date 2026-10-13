// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The publish target for the landing site (agentmux.ai). Unlike the docs, the
// landing site names its images for where they go (`ui-agent.webp`), not for their
// shot, and serves WebP and MP4. So it keeps a manifest,
// public/screenshots/manifest.json, that says which shot or clip each file
// comes from and how to encode it:
//
//   { "images": [
//       { "file": "ui-agent.webp", "shot": "agent-conversation", "width": 960, "quality": 82 },
//       { "file": "ui-tower.webp", "shot": "widget-tower", "size": "small" },
//       { "file": "ui-swarm-follows.mp4", "clip": "swarm-follows-agent", "width": 1280,
//         "poster": "ui-swarm-follows-poster.webp" } ] }
//
// Each entry is encoded from the run's PNG (a shot) or MP4 (a clip) with
// ffmpeg. Only files the manifest lists are written; a shot not marked
// publishable is skipped, as for the docs (publish.mjs runImages).

import { copyFileSync, existsSync, mkdtempSync, readFileSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, extname, join, relative } from "node:path";
import { spawnSync } from "node:child_process";
import { runImages } from "./publish.mjs";

/** A manifest file name: a bare name in public/screenshots, nothing that walks out of it. */
const SAFE_NAME = /^[a-z0-9][a-z0-9._-]*\.(webp|mp4)$/;

/** Validates the manifest; returns its entries or throws, listing every problem. */
export function readManifest(manifest) {
    const errors = [];
    const entries = manifest?.images;
    if (!Array.isArray(entries)) throw new Error("manifest.json: `images` must be an array");
    const seen = new Set();
    entries.forEach((e, i) => {
        const at = `images[${i}] (${e?.file ?? "?"})`;
        const names = [e?.file, e?.poster].filter(Boolean);
        for (const n of names) {
            if (!SAFE_NAME.test(n)) errors.push(`${at}: "${n}" isn't a plain .webp or .mp4 name`);
            if (seen.has(n)) errors.push(`${at}: "${n}" is listed twice`);
            seen.add(n);
        }
        if (!!e?.shot === !!e?.clip) errors.push(`${at}: give exactly one of "shot" or "clip"`);
        if (e?.shot && extname(e.file ?? "") !== ".webp") errors.push(`${at}: a shot is published as .webp`);
        if (e?.clip && extname(e.file ?? "") !== ".mp4") errors.push(`${at}: a clip is published as .mp4`);
        if (e?.poster && !e?.clip) errors.push(`${at}: only a clip has a poster`);
        if (e?.poster && extname(e.poster) !== ".webp") errors.push(`${at}: a poster is .webp`);
    });
    if (errors.length) throw new Error(`manifest.json:\n  ${errors.join("\n  ")}`);
    return entries;
}

/** The run file an entry is encoded from: the shot's PNG (at `size`, when the
 *  shot was taken at several), or the clip's MP4. */
function sourceFor(entry, images) {
    if (entry.shot) {
        const name = `${entry.size ? `${entry.shot}-${entry.size}` : entry.shot}.png`;
        return images.filter((i) => i.name === name);
    }
    return images.filter((i) => i.id === entry.clip && i.name.endsWith(".mp4"));
}

/** What publishing would do for each entry: `encode`, or `skipped` / `missing`
 *  / `ambiguous` (two runs have it) with `why`. */
export function landingPlan(entries, images) {
    return entries.map((entry) => {
        const sources = sourceFor(entry, images);
        const what = entry.shot ? `shot ${entry.shot}${entry.size ? ` (${entry.size})` : ""}` : `clip ${entry.clip} (.mp4)`;
        if (sources.length > 1) return { entry, action: "ambiguous", why: `more than one run has ${what}` };
        if (sources.length === 0) return { entry, action: "missing", why: `no run has ${what}` };
        const [src] = sources;
        if (src.machineData) return { entry, action: "skipped", why: `${what} shows this machine's data, or isn't marked publishable` };
        if (!existsSync(src.source)) return { entry, action: "missing", why: `${what} is in the manifest but not on disk` };
        return { entry, action: "encode", source: src.source };
    });
}

/** The ffmpeg runs that produce an entry's file(s) from `source` into `dir`. */
export function encodeCommands(entry, source, dir) {
    const scale = entry.width ? [`scale=${entry.width}:-2:flags=lanczos`] : [];
    const quiet = ["-hide_banner", "-loglevel", "error", "-y"];
    const webp = (vf, out) => [...quiet, "-i", source, ...(vf.length ? ["-vf", vf.join(",")] : []), "-frames:v", "1", "-c:v", "libwebp", "-quality", String(entry.quality ?? 82), "-compression_level", "6", out];
    if (entry.shot) return [{ file: entry.file, args: webp(scale, join(dir, entry.file)) }];
    // H.264 in yuv420p needs even dimensions; muted (there's no audio track), and
    // faststart so it plays before it has fully downloaded.
    const vf = [...scale, "scale=trunc(iw/2)*2:trunc(ih/2)*2", "format=yuv420p"].join(",");
    const cmds = [{ file: entry.file, args: [...quiet, "-i", source, "-an", "-vf", vf, "-c:v", "libx264", "-crf", String(entry.crf ?? 26), "-preset", "slow", "-movflags", "+faststart", join(dir, entry.file)] }];
    // The poster is the clip's first frame, so the page doesn't jump when it starts playing.
    if (entry.poster) cmds.push({ file: entry.poster, args: webp(scale, join(dir, entry.poster)) });
    return cmds;
}

function sameBytes(a, b) {
    return existsSync(b) && statSync(a).size === statSync(b).size && readFileSync(a).equals(readFileSync(b));
}

/** Publishes the runs in `from` into the landing clone at `landing`; returns
 *  the plan. `exec` runs ffmpeg (tests pass a stand-in). */
export function publishLanding({ landing, from, ffmpeg, dryRun = false, log = console.log, exec = spawnSync }) {
    const shotsDir = join(landing, "public", "screenshots");
    const manifestPath = join(shotsDir, "manifest.json");
    if (!existsSync(manifestPath)) throw new Error(`${landing} doesn't look like an agentmux-landing clone (no public/screenshots/manifest.json)`);
    const entries = readManifest(JSON.parse(readFileSync(manifestPath, "utf8")));
    const images = [];
    for (const dir of from) {
        const m = join(dir, "manifest.json");
        if (!existsSync(m)) throw new Error(`no manifest.json in ${dir}`);
        images.push(...runImages(JSON.parse(readFileSync(m, "utf8")), dir));
    }
    const steps = landingPlan(entries, images);
    if (steps.some((s) => s.action === "encode") && !ffmpeg) throw new Error("ffmpeg is needed to encode the landing images");
    const tmp = mkdtempSync(join(tmpdir(), "amlanding-"));
    let changed = 0;
    try {
        for (const s of steps) {
            if (s.action !== "encode") {
                log(`${s.action === "ambiguous" ? "AMBIGUOUS" : s.action}  ${s.entry.file} (${s.why})`);
                continue;
            }
            for (const c of encodeCommands(s.entry, s.source, tmp)) {
                const r = exec(ffmpeg, c.args, { encoding: "utf8" });
                if (r.status !== 0) throw new Error(`ffmpeg failed for ${c.file}: ${r.stderr || r.error?.message}`);
                const out = join(tmp, c.file);
                const target = join(shotsDir, c.file);
                const kb = Math.round(statSync(out).size / 1024);
                if (sameBytes(out, target)) { log(`unchanged  ${c.file} (${kb} KB)`); continue; }
                if (!dryRun) copyFileSync(out, target);
                changed++;
                log(`${dryRun ? "would update" : "updated"}  ${relative(landing, target)} (${kb} KB, from ${basename(s.source)})`);
            }
        }
    } finally {
        rmSync(tmp, { recursive: true, force: true });
    }
    const count = (a) => steps.filter((s) => s.action === a).length;
    log(`\n${changed} file(s) ${dryRun ? "would change" : "changed"}; ${count("encode")} encoded, ${count("missing")} missing, ${count("skipped")} skipped, ${count("ambiguous")} ambiguous`);
    return steps;
}
