#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-audio-creators.mjs — nothing in the app makes a sound except through the sound service
// (docs/specs/SPEC_ATTENTION_TOAST_WITH_LINK_TO_AGENT_2026_11_02.md §3.8). Every sound there is classified
// (`SOUNDS[id].source`): an ask plays only for a request in the waiting registry, which also gets the toast and
// the list row; an event carries the pane it flashes. A beep made with `new Audio` or an `AudioContext`
// elsewhere would have neither, so it could not be found when heard.
//
// Fails on `new Audio(` or `new AudioContext(` outside `frontend/app/notification/sound/`. Voice input listens to
// the microphone and plays nothing, so its two files are allowed by path.
//
//   node scripts/check-audio-creators.mjs

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

// fileURLToPath, not `.pathname`: on Windows the latter is "/C:/…".
const ROOT = fileURLToPath(new URL("..", import.meta.url));
const ALLOWED = [
    "frontend/app/notification/sound/",
    "frontend/app/hook/useMicLevelMeter.ts",
    "frontend/app/hook/whisperVoiceEngine.ts",
];
const CREATOR = /\bnew\s+(?:Audio|(?:window\.)?(?:AudioContext|webkitAudioContext))\s*\(/;

function* sources(dir) {
    for (const name of readdirSync(dir)) {
        if (name === "node_modules" || name.startsWith(".")) continue;
        const path = join(dir, name);
        if (statSync(path).isDirectory()) yield* sources(path);
        else if (/\.(ts|tsx)$/.test(name) && !/\.test\.(ts|tsx)$/.test(name)) yield path;
    }
}

const problems = [];
for (const path of sources(join(ROOT, "frontend"))) {
    const rel = relative(ROOT, path).replaceAll("\\", "/");
    if (ALLOWED.some((a) => rel.startsWith(a))) continue;
    readFileSync(path, "utf8")
        .split("\n")
        .forEach((line, i) => {
            if (CREATOR.test(line.replace(/\/\/.*$/, ""))) problems.push(`${rel}:${i + 1}: ${line.trim()}`);
        });
}

if (problems.length > 0) {
    console.error("check-audio-creators: sound made outside the sound service:");
    for (const p of problems) console.error(`  ${p}`);
    console.error("Add the sound to frontend/app/notification/sound/sounds.ts (with its `source`) and play it through the service.");
    process.exit(1);
}
console.log("check-audio-creators: ok");
