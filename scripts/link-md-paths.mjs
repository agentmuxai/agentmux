#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// link-md-paths.mjs — turns a backticked repo path in a Markdown file into a
// relative link, so a reader on GitHub can click through to the file.
//
//   `crates/srv/src/main.rs`      ->  [`crates/srv/src/main.rs`](crates/srv/src/main.rs)
//   `scripts/release.sh:183`      ->  [`scripts/release.sh:183`](scripts/release.sh#L183)
//
//   node scripts/link-md-paths.mjs FILE...      rewrite these files
//   node scripts/link-md-paths.mjs --all        rewrite every tracked .md file
//   node scripts/link-md-paths.mjs --check ...  rewrite nothing; list what would
//                                               change, exit 1 if anything would
//
// Only a path that is tracked in git (or a folder holding tracked files) is
// linked, so the link works on GitHub, not just in this checkout. Written
// relative to the Markdown file, which is what GitHub and
// scripts/check-doc-links.mjs both resolve against.
//
// Left alone: code in fenced blocks, code already inside a link, code in a
// heading, paths that don't exist (stale ones are for a person to fix), and
// bare file names (`mod.rs`) in a file below the repo root, where they usually
// mean a file next to the code being discussed, not a file at the root.
// A `:N` or `:N-M` suffix becomes a `#LN` / `#LN-LM` anchor, except on a .md
// target, which GitHub renders and so has no line anchors.
// The agentmux-docs site does the same at build time with remark plugins; this repo
// is read on GitHub as written, so the links have to be in the source.

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { unified } from "unified";
import remarkParse from "remark-parse";
import remarkGfm from "remark-gfm";

const PATH_RE = /^(?:\.\/)?[A-Za-z0-9_.@-]+(?:\/[A-Za-z0-9_.@-]+)*\/?(?::(\d+)(?:[-–](\d+))?)?$/;
const SKIP_ANCESTORS = new Set(["link", "linkReference", "heading", "definition"]);
// Written only by the release flow (scripts/release.sh), never by hand in a feature PR.
const NOT_OURS = (f) => f === "VERSION_HISTORY.md" || f.startsWith(".changesets/");

/** Index of tracked files and the folders that hold them. */
export function trackedIndex(repoRoot) {
  const files = execFileSync("git", ["ls-files"], { cwd: repoRoot, encoding: "utf8" })
    .split("\n")
    .filter(Boolean);
  const all = new Set(files);
  for (const f of files) {
    let d = path.posix.dirname(f);
    while (d !== "." && !all.has(d)) {
      all.add(d);
      d = path.posix.dirname(d);
    }
  }
  return all;
}

/** The link target for a backticked value in `mdFile`, or null if it isn't a tracked path. */
export function resolveTarget(value, mdFile, tracked) {
  const m = PATH_RE.exec(value);
  if (!m) return null;
  const [, start, end] = m;
  const p = value
    .replace(/:\d+(?:[-–]\d+)?$/, "")
    .replace(/^\.\//, "")
    .replace(/\/$/, "");
  if (!p || p === "." || p === "..") return null;
  const mdDir = path.posix.dirname(mdFile);
  // `frontend/` names a folder as surely as `frontend/app` does.
  const hasSlash = p.includes("/") || /\/(?::\d|$)/.test(value);
  // A bare name below the root is ambiguous; only a root-level doc links one.
  if (!hasSlash && mdDir !== ".") return null;
  // A bare name must look like a file, not a word that happens to be a folder.
  if (!hasSlash && !/\.[A-Za-z0-9]+$/.test(p)) return null;

  let repoPath = null;
  if (tracked.has(p)) repoPath = p;
  else if (hasSlash && mdDir !== ".") {
    const rel = path.posix.normalize(path.posix.join(mdDir, p));
    if (!rel.startsWith("..") && tracked.has(rel)) repoPath = rel;
  }
  if (!repoPath) return null;

  let target = path.posix.relative(mdDir === "." ? "" : mdDir, repoPath) || ".";
  if (start && !repoPath.endsWith(".md")) {
    target += end ? `#L${start}-L${end}` : `#L${start}`;
  }
  return target;
}

/** Rewrite one Markdown text. Returns { text, changes: [{ line, value, target }] }. */
export function linkPaths(text, mdFile, tracked) {
  const tree = unified().use(remarkParse).use(remarkGfm).parse(text);
  // YAML frontmatter (skills' SKILL.md) is metadata, not prose; remark without
  // a frontmatter plugin parses it as text, so keep edits out of it.
  const fm = /^---\r?\n[\s\S]*?\r?\n---\r?\n/.exec(text);
  const bodyStart = fm ? fm[0].length : 0;
  const edits = [];
  const walk = (node) => {
    if (SKIP_ANCESTORS.has(node.type)) return;
    if (node.type === "inlineCode" && node.position && node.position.start.offset >= bodyStart) {
      const target = resolveTarget(node.value, mdFile, tracked);
      if (target) {
        edits.push({
          start: node.position.start.offset,
          end: node.position.end.offset,
          line: node.position.start.line,
          value: node.value,
          target,
        });
      }
      return;
    }
    for (const child of node.children ?? []) walk(child);
  };
  walk(tree);
  let out = text;
  for (const e of [...edits].sort((a, b) => b.start - a.start)) {
    const code = out.slice(e.start, e.end);
    out = `${out.slice(0, e.start)}[${code}](${e.target})${out.slice(e.end)}`;
  }
  return { text: out, changes: edits.map(({ line, value, target }) => ({ line, value, target })) };
}

function main(argv) {
  const repoRoot = execFileSync("git", ["rev-parse", "--show-toplevel"], { encoding: "utf8" }).trim();
  const check = argv.includes("--check");
  const all = argv.includes("--all");
  const tracked = trackedIndex(repoRoot);
  let files = argv.filter((a) => !a.startsWith("--"));
  if (all) files = [...tracked].filter((f) => f.endsWith(".md"));
  files = files.map((f) => path.relative(repoRoot, path.resolve(f)).split(path.sep).join("/"));
  if (!files.length) {
    console.error("usage: link-md-paths.mjs [--check] (--all | FILE...)");
    return 2;
  }

  let total = 0;
  let touched = 0;
  for (const f of files) {
    if (!f.endsWith(".md") || !tracked.has(f) || NOT_OURS(f)) continue;
    const abs = path.join(repoRoot, f);
    const before = readFileSync(abs, "utf8");
    const { text, changes } = linkPaths(before, f, tracked);
    if (!changes.length) continue;
    total += changes.length;
    touched++;
    if (check) for (const c of changes) console.log(`${f}:${c.line}: \`${c.value}\` -> ${c.target}`);
    else writeFileSync(abs, text);
  }
  console.log(
    `link-md-paths: ${total} path(s) ${check ? "to link" : "linked"} in ${touched} file(s) of ${files.length}`,
  );
  return check && total ? 1 : 0;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main(process.argv.slice(2));
}
