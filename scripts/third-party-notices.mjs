// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Third-party notices for the shipped app.
//
// Most licences of the software AgentMux bundles (MIT, BSD, ISC, Apache-2.0,
// OFL, Unicode, ...) require their text to go with every copy, binary copies
// included. This builds one THIRD_PARTY_NOTICES.txt from what a build actually
// ships, and the Vite plugin below writes it, with AgentMux's own LICENSE and
// NOTICE, into dist/frontend/licenses/. Every packager copies dist/frontend
// whole, so the files land in every package format.
//
// - JavaScript: the packages whose modules are in the bundle (Rollup's module
//   list, so build-only and unused dependencies are left out).
// - Rust: every crate our workspace reaches through normal dependencies, on
//   any platform (`cargo metadata`); build-only and proc-macro crates aren't
//   in the binaries and are left out. Metadata's graph also has a few crates
//   that only an unused feature or target pulls in; listing those is harmless.
// - licenses/bundled/: fonts and tools that come from neither (checked in).
//
// For each package: the licence we use it under (when it offers a choice, the
// first of PREFERENCE), its licence files for that licence, and any NOTICE
// file. A package that ships no licence file gets the standard text from
// licenses/templates/ with its declared authors.
//
// A release build (RELEASE_CHANNEL set) fails if any of this can't be done;
// a local build warns and writes what it could.

import { execFileSync } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import process from "node:process";

const PREFERENCE = [
  "MIT",
  "Apache-2.0",
  "BSD-3-Clause",
  "BSD-2-Clause",
  "ISC",
  "Zlib",
  "0BSD",
  "Unlicense",
  "CC0-1.0",
  "BSL-1.0",
  "Unicode-3.0",
  "MPL-2.0",
];

// Words in a licence file's name that tie it to one licence.
const FILE_KEYWORDS = {
  MIT: /mit/i,
  "Apache-2.0": /apache/i,
  "BSD-3-Clause": /bsd/i,
  "BSD-2-Clause": /bsd/i,
  ISC: /isc/i,
  Zlib: /zlib/i,
  "0BSD": /0bsd/i,
  Unlicense: /unlicense/i,
  "CC0-1.0": /cc0/i,
  "BSL-1.0": /boost|bsl/i,
  "Unicode-3.0": /unicode/i,
  "MPL-2.0": /mpl|mozilla/i,
};
const ANY_KEYWORD = /mit|apache|bsd|isc|zlib|unlicense|cc0|boost|bsl|unicode|mpl|mozilla|lgpl|gpl/i;
const LICENCE_FILE = /^(licen[cs]e|copying|unlicense|notice|copyright)/i;
const NOTICE_FILE = /^notice/i;

function rank(id) {
  const base = id.split(" WITH ")[0];
  const i = PREFERENCE.indexOf(base);
  return i < 0 ? 100 : i + (id.includes(" WITH ") ? 0.5 : 0);
}

/** Parse an SPDX expression (also the old "MIT/Apache-2.0" form) into a tree. */
export function parseSpdx(expr) {
  const tokens = String(expr ?? "")
    .replace(/\//g, " OR ")
    .replace(/\(/g, " ( ")
    .replace(/\)/g, " ) ")
    .split(/\s+/)
    .filter(Boolean);
  let pos = 0;
  const peek = () => tokens[pos];
  function atom() {
    if (peek() === "(") {
      pos++;
      const inner = or();
      if (tokens[pos++] !== ")") throw new Error(`unbalanced licence expression: ${expr}`);
      return inner;
    }
    let id = tokens[pos++];
    if (id == null || ["AND", "OR", "WITH", ")"].includes(id)) {
      throw new Error(`bad licence expression: ${expr}`);
    }
    if (peek() === "WITH") {
      pos++;
      id = `${id} WITH ${tokens[pos++]}`;
    }
    return { id };
  }
  function and() {
    const parts = [atom()];
    while (peek() === "AND") {
      pos++;
      parts.push(atom());
    }
    return parts.length === 1 ? parts[0] : { and: parts };
  }
  function or() {
    const parts = [and()];
    while (peek() === "OR") {
      pos++;
      parts.push(and());
    }
    return parts.length === 1 ? parts[0] : { or: parts };
  }
  const tree = or();
  if (pos !== tokens.length) throw new Error(`bad licence expression: ${expr}`);
  return tree;
}

/** The licences we take a package under: every AND term, the preferred OR branch. */
export function chooseLicences(expr) {
  function pick(node) {
    if (node.id) return [node.id];
    if (node.and) return [...new Set(node.and.flatMap(pick))];
    const options = node.or.map(pick);
    const score = (ids) => Math.max(...ids.map(rank)) * 100 + ids.length;
    return options.reduce((best, o) => (score(o) < score(best) ? o : best));
  }
  return pick(parseSpdx(expr));
}

/** Licence and notice files at the top of a package (and in a LICENSES/ folder). */
export function listLicenceFiles(dir) {
  let entries;
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return [];
  }
  const out = [];
  for (const e of entries) {
    if (!LICENCE_FILE.test(e.name)) continue;
    const full = path.join(dir, e.name);
    if (e.isFile()) out.push(full);
    else if (e.isDirectory() && /^licenses?$/i.test(e.name)) {
      for (const f of fs.readdirSync(full)) {
        if (fs.statSync(path.join(full, f)).isFile()) out.push(path.join(full, f));
      }
    }
  }
  return out.sort();
}

/** Of a package's licence files, the ones for the licences we chose, plus every NOTICE. */
export function selectLicenceFiles(files, chosen) {
  const name = (f) => path.basename(f);
  const notices = files.filter((f) => NOTICE_FILE.test(name(f)));
  const texts = files.filter((f) => !NOTICE_FILE.test(name(f)));
  const keywords = chosen.map((id) => FILE_KEYWORDS[id.split(" WITH ")[0]]).filter(Boolean);
  const specific = texts.filter((f) => keywords.some((k) => k.test(name(f))));
  let picked = specific;
  if (picked.length === 0) {
    const generic = texts.filter((f) => !ANY_KEYWORD.test(name(f)));
    picked = generic.length > 0 ? generic : texts;
  }
  return [...picked, ...notices];
}

function readText(file) {
  return fs.readFileSync(file, "utf8").replace(/\r\n/g, "\n").trim() + "\n";
}

function apacheTerms(repoRoot) {
  const text = readText(path.join(repoRoot, "LICENSE"));
  const end = text.indexOf("END OF TERMS AND CONDITIONS");
  return end > 0 ? text.slice(0, end + "END OF TERMS AND CONDITIONS".length) + "\n" : text;
}

/** Standard text for a licence, for a package that ships none. */
function templateText(repoRoot, id, holders) {
  if (id === "Apache-2.0") return apacheTerms(repoRoot);
  const file = path.join(repoRoot, "licenses", "templates", `${id}.txt`);
  if (!fs.existsSync(file)) return null;
  return readText(file).replace("{holders}", holders);
}

/**
 * Notice text for one package: its chosen licence files and NOTICE files, or
 * the standard texts. Returns { text, used, problem }.
 */
export function packageNoticeText(repoRoot, pkg) {
  // A crate with its own licence file (Cargo's license-file) names no SPDX id.
  if (pkg.licenseFile && fs.existsSync(pkg.licenseFile)) {
    const own = path.resolve(pkg.licenseFile);
    const notices = listLicenceFiles(pkg.dir).filter(
      (f) => NOTICE_FILE.test(path.basename(f)) && path.resolve(f) !== own
    );
    return { text: [own, ...notices].map(readText).join("\n"), used: [], problem: null };
  }
  let used;
  try {
    used = pkg.license ? chooseLicences(pkg.license) : null;
  } catch {
    used = null;
  }
  const files = selectLicenceFiles(listLicenceFiles(pkg.dir), used ?? []);
  if (files.length > 0) {
    return { text: files.map(readText).join("\n"), used: used ?? [], problem: null };
  }
  if (!used) {
    return { text: "", used: [], problem: `no licence file and no usable licence (${pkg.license ?? "none"})` };
  }
  const holders = pkg.authors?.length ? pkg.authors.join(", ") : `the ${pkg.name} authors`;
  const parts = [];
  for (const id of used) {
    const t = templateText(repoRoot, id.split(" WITH ")[0], holders);
    if (t == null) return { text: "", used, problem: `no licence file, and no standard text for ${id}` };
    parts.push(t);
  }
  return { text: parts.join("\n"), used, problem: null };
}

/** A package.json repository field as a browsable URL. */
export function repoUrl(repo) {
  return repo
    .replace(/^git\+/, "")
    .replace(/^git:\/\//, "https://")
    .replace(/^ssh:\/\/git@/, "https://")
    .replace(/^git@([^:]+):/, "https://$1/")
    .replace(/^github:/, "https://github.com/")
    .replace(/^([\w.-]+\/[\w.-]+)$/, "https://github.com/$1")
    .replace(/\.git$/, "");
}

/** npm packages behind a list of bundled module ids. */
export function jsPackagesFromModules(moduleIds) {
  const dirs = new Set();
  for (const raw of moduleIds) {
    const id = raw.replace(/^\0/, "").split("?")[0].replace(/\\/g, "/");
    const i = id.lastIndexOf("/node_modules/");
    if (i < 0) continue;
    const rest = id.slice(i + "/node_modules/".length).split("/");
    const name = rest[0].startsWith("@") ? `${rest[0]}/${rest[1]}` : rest[0];
    if (!name || name.startsWith(".")) continue;
    dirs.add(id.slice(0, i + "/node_modules/".length) + name);
  }
  const out = [];
  for (const dir of [...dirs].sort()) {
    let pj;
    try {
      pj = JSON.parse(fs.readFileSync(path.join(dir, "package.json"), "utf8"));
    } catch {
      continue;
    }
    const license = typeof pj.license === "string" ? pj.license : (pj.license?.type ?? null);
    const person = (p) => (typeof p === "string" ? p.replace(/\s*[<(].*$/, "") : p?.name);
    const authors = [pj.author, ...(pj.contributors ?? [])].map(person).filter(Boolean);
    const repo = typeof pj.repository === "string" ? pj.repository : pj.repository?.url;
    out.push({
      kind: "npm",
      name: pj.name ?? path.basename(dir),
      version: pj.version ?? "",
      license,
      authors,
      dir,
      source: repo ? repoUrl(repo) : `https://www.npmjs.com/package/${pj.name}`,
    });
  }
  return out;
}

/** Third-party crates the workspace's binaries contain, on any platform. */
export function rustCrates(repoRoot) {
  const meta = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--locked"], {
      cwd: repoRoot,
      maxBuffer: 512 * 1024 * 1024,
      stdio: ["ignore", "pipe", "pipe"],
    }).toString()
  );
  const pkgs = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const members = new Set(meta.workspace_members);
  const seen = new Set(members);
  const queue = [...members];
  while (queue.length > 0) {
    const node = nodes.get(queue.shift());
    for (const dep of node?.deps ?? []) {
      if (seen.has(dep.pkg)) continue;
      if (!dep.dep_kinds.some((k) => k.kind == null)) continue;
      const p = pkgs.get(dep.pkg);
      if (p.targets.some((t) => t.kind.includes("proc-macro"))) continue;
      seen.add(dep.pkg);
      queue.push(dep.pkg);
    }
  }
  const out = [];
  for (const id of seen) {
    if (members.has(id)) continue;
    const p = pkgs.get(id);
    out.push({
      kind: "crate",
      name: p.name,
      version: p.version,
      license: p.license,
      authors: (p.authors ?? []).map((a) => a.replace(/\s*<.*$/, "")),
      dir: path.dirname(p.manifest_path),
      source: p.repository ?? `https://crates.io/crates/${p.name}`,
      licenseFile: p.license_file ? path.join(path.dirname(p.manifest_path), p.license_file) : null,
    });
  }
  return out.sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
}

const RULE = "=".repeat(78);
const THIN = "-".repeat(78);

/** Group packages by identical notice text, so a shared licence prints once. */
function renderSection(repoRoot, title, packages, problems) {
  const groups = new Map();
  for (const pkg of packages) {
    const notice = packageNoticeText(repoRoot, pkg);
    if (notice.problem) problems.push(`${pkg.name} ${pkg.version}: ${notice.problem}`);
    const key = notice.text;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push({ pkg, used: notice.used });
  }
  const lines = [RULE, title, RULE, ""];
  for (const [text, members] of groups) {
    for (const { pkg, used } of members) {
      const choice = used.length && used.join(" AND ") !== pkg.license ? ` (used under ${used.join(" AND ")})` : "";
      lines.push(`${pkg.name} ${pkg.version}: ${pkg.license ?? "no licence declared"}${choice}. ${pkg.source}`);
    }
    lines.push(THIN, text.trimEnd() || "(no licence text found)", "", "");
  }
  return lines.join("\n");
}

function bundledSection(repoRoot) {
  const dir = path.join(repoRoot, "licenses", "bundled");
  const parts = [RULE, "Bundled fonts, tools and data", RULE, ""];
  for (const f of fs.readdirSync(dir).sort()) {
    parts.push(readText(path.join(dir, f)), THIN, "");
  }
  parts.push(
    "Font Awesome Free (fontawesome/ in this folder's parent): icons CC BY 4.0,",
    "fonts SIL OFL 1.1, code MIT. Full text: fontawesome/LICENSE.txt.",
    ""
  );
  return parts.join("\n");
}

/** The full THIRD_PARTY_NOTICES.txt. Returns { text, problems }. */
export function buildNotices({ repoRoot, jsPackages, rustPackages, rustError }) {
  const version = JSON.parse(fs.readFileSync(path.join(repoRoot, "package.json"), "utf8")).version;
  const problems = [];
  const header = [
    `AgentMux ${version}: third-party software notices`,
    "",
    "AgentMux is Copyright AgentMux Corp. and licensed under the Apache License 2.0;",
    "see LICENSE.txt and NOTICE.txt in this folder. It includes the third-party",
    "software below, each under its own licence. Where a package offers a choice",
    "of licences, the one AgentMux uses it under is noted.",
    "",
    `  1. Bundled fonts, tools and data`,
    `  2. JavaScript packages in the user interface (${jsPackages.length})`,
    `  3. Rust crates in the AgentMux programs (${rustPackages ? rustPackages.length : "not generated"})`,
    "",
    "",
  ].join("\n");
  const rust = rustPackages
    ? renderSection(repoRoot, "Rust crates in the AgentMux programs", rustPackages, problems)
    : [RULE, "Rust crates in the AgentMux programs", RULE, "", `Not generated in this build: ${rustError}`, ""].join(
        "\n"
      );
  const text = [
    header,
    bundledSection(repoRoot),
    renderSection(repoRoot, "JavaScript packages in the user interface", jsPackages, problems),
    rust,
  ].join("\n");
  return { text, problems };
}

/**
 * Vite plugin: write licenses/ into the build output.
 * @returns {import("vite").Plugin}
 */
export function thirdPartyNotices({ repoRoot, strict = Boolean(process.env.RELEASE_CHANNEL) }) {
  return {
    name: "agentmux-third-party-notices",
    apply: "build",
    generateBundle(_options, bundle) {
      const moduleIds = [];
      for (const file of Object.values(bundle)) {
        if (file.type === "chunk") moduleIds.push(...Object.keys(file.modules));
      }
      const jsPackages = jsPackagesFromModules(moduleIds);
      let rustPackages = null;
      let rustError = null;
      try {
        rustPackages = rustCrates(repoRoot);
      } catch (e) {
        rustError = String(e?.stderr ?? e?.message ?? e).split("\n")[0];
      }
      const { text, problems } = buildNotices({ repoRoot, jsPackages, rustPackages, rustError });
      if (rustError) problems.push(`Rust crates not listed: ${rustError}`);
      if (problems.length > 0) {
        const msg = `third-party notices:\n  ${problems.join("\n  ")}`;
        if (strict) this.error(msg);
        this.warn(msg);
      }
      const emit = (fileName, source) => this.emitFile({ type: "asset", fileName, source });
      emit("licenses/THIRD_PARTY_NOTICES.txt", text);
      emit("licenses/LICENSE.txt", fs.readFileSync(path.join(repoRoot, "LICENSE")));
      emit("licenses/NOTICE.txt", fs.readFileSync(path.join(repoRoot, "NOTICE")));
    },
  };
}
