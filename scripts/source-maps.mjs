// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The frontend's source maps: check that a build has them, and decode a minified stack with them.
//
//   node scripts/source-maps.mjs check <dist/frontend>
//       Every built .js that names a map (`//# sourceMappingURL=`) has that map beside it, and each map parses,
//       says where its sources are (relative ones: no build machine's absolute path) and has some. Run it on a
//       build before its maps are stripped from an installer or packed into the release's maps archive.
//   node scripts/source-maps.mjs resolve <maps dir> [stack file]
//       Decode a stack read from the file, or from stdin. The maps dir is a dist/frontend, or the unpacked maps
//       archive of a release (the same layout: assets/<chunk>.js.map).
//
// The runtime resolver (frontend/log/source-map-resolver.ts) does the same for a stack as it is logged, but only
// where the maps are installed; a release's installers leave them out to save ~28 MB, so a stack from an installed
// build is decoded here, against the maps archive published with that release.

import fs from "node:fs";
import path from "node:path";
import { SourceMapConsumer } from "source-map-js";
import { isMain } from "./lib/is-main.mjs";

const MAP_COMMENT = /\/\/# sourceMappingURL=(\S+)\s*$/;
const ABSOLUTE = /^(?:[A-Za-z]:[\\/]|\/(?!\/)|file:)/;

function* jsFiles(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) yield* jsFiles(full);
    else if (entry.name.endsWith(".js")) yield full;
  }
}

/** What is wrong with the maps of the build in `dir`, one line each; an empty list is a build that is fine. */
export function checkMaps(dir) {
  const problems = [];
  let maps = 0;
  for (const js of jsFiles(dir)) {
    const tail = fs.readFileSync(js, "utf8").slice(-300);
    const m = tail.match(MAP_COMMENT);
    if (!m) continue;
    const rel = path.relative(dir, js);
    const mapPath = path.join(path.dirname(js), m[1]);
    if (!fs.existsSync(mapPath)) {
      problems.push(`${rel}: names ${m[1]}, which is not there`);
      continue;
    }
    let json;
    try {
      json = JSON.parse(fs.readFileSync(mapPath, "utf8"));
    } catch (e) {
      problems.push(`${rel}: ${m[1]} is not JSON (${e.message})`);
      continue;
    }
    // A chunk that is only an inlined asset (an svg as a string) has a map with no sources and no mappings: fine.
    const assetOnly = Array.isArray(json.sources) && json.sources.length === 0 && json.mappings === "";
    if (
      json.version !== 3 ||
      !Array.isArray(json.sources) ||
      typeof json.mappings !== "string" ||
      (json.sources.length === 0 && !assetOnly)
    ) {
      problems.push(`${rel}: ${m[1]} is not a source map with sources`);
      continue;
    }
    const absolute = json.sources.find((s) => ABSOLUTE.test(s));
    if (absolute) problems.push(`${rel}: ${m[1]} has an absolute source path (${absolute})`);
    maps++;
  }
  if (maps === 0) problems.push("no chunk names a source map: the build has none");
  return problems;
}

const FRAME = /^(\s*at\s+)(?:(.+?)\s+)?\(?([^()\s]+?):(\d+):(\d+)\)?\s*$/;

/** `stack` with every frame in a chunk that has a map in `dir` rewritten to its original source, line and name. */
export function resolveStack(dir, stack) {
  const cache = new Map();
  const consumerFor = (chunk) => {
    if (cache.has(chunk)) return cache.get(chunk);
    let found = null;
    for (const candidate of [path.join(dir, `${chunk}.map`), path.join(dir, "assets", `${chunk}.map`)]) {
      if (fs.existsSync(candidate)) {
        found = new SourceMapConsumer(JSON.parse(fs.readFileSync(candidate, "utf8")));
        break;
      }
    }
    cache.set(chunk, found);
    return found;
  };
  return stack
    .split(/\r?\n/)
    .map((line) => {
      const m = line.match(FRAME);
      if (!m) return line;
      const [, prefix, fn, url, l, c] = m;
      const chunk = url.split("?")[0].split("/").pop();
      const consumer = consumerFor(chunk);
      if (!consumer) return `${line}  [no map for ${chunk}]`;
      // V8 columns are 1-based, the parser's are 0-based.
      const o = consumer.originalPositionFor({ line: Number(l), column: Math.max(0, Number(c) - 1) });
      if (!o.source) return `${line}  [no mapping]`;
      return `${prefix}${o.name ?? fn ?? "(anonymous)"} (${o.source}:${o.line}:${(o.column ?? 0) + 1})`;
    })
    .join("\n");
}

function main(argv) {
  const [cmd, dir, file] = argv;
  if (cmd === "check" && dir) {
    const problems = checkMaps(dir);
    if (problems.length > 0) {
      console.error(`ERROR: the source maps of ${dir} are not all there:\n  ${problems.join("\n  ")}`);
      return 1;
    }
    console.log(`OK: every chunk of ${dir} that names a source map has it, with relative sources.`);
    return 0;
  }
  if (cmd === "resolve" && dir) {
    const text = fs.readFileSync(file ?? 0, "utf8");
    console.log(resolveStack(dir, text));
    return 0;
  }
  console.error("usage: node scripts/source-maps.mjs check <dist/frontend> | resolve <maps dir> [stack file]");
  return 2;
}

if (isMain(import.meta.url)) {
  process.exitCode = main(process.argv.slice(2));
}
