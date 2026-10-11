// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for third-party-notices.mjs: which licence a package is used
// under, which of its files carry that licence, and the standard-text fallback
// for packages that ship no licence file.

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import {
  chooseLicences,
  chromiumCredits,
  jsPackagesFromModules,
  packageNoticeText,
  parseNoticeSections,
  parseSpdx,
  repoUrl,
  selectLicenceFiles,
  shikiAssetsFromModules,
  shikiNotices,
} from "./third-party-notices.mjs";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

describe("chooseLicences", () => {
  it("takes MIT from a dual licence, in either spelling", () => {
    expect(chooseLicences("MIT OR Apache-2.0")).toEqual(["MIT"]);
    expect(chooseLicences("Apache-2.0 OR MIT")).toEqual(["MIT"]);
    expect(chooseLicences("MIT/Apache-2.0")).toEqual(["MIT"]);
  });

  it("keeps every AND term", () => {
    expect(chooseLicences("Apache-2.0 AND ISC")).toEqual(["Apache-2.0", "ISC"]);
    expect(chooseLicences("(MIT OR Apache-2.0) AND Unicode-3.0")).toEqual(["MIT", "Unicode-3.0"]);
  });

  it("prefers a permissive option over LGPL or an exception", () => {
    expect(chooseLicences("MIT OR Apache-2.0 OR LGPL-2.1-or-later")).toEqual(["MIT"]);
    expect(chooseLicences("Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT")).toEqual(["MIT"]);
    expect(chooseLicences("(MPL-2.0 OR Apache-2.0)")).toEqual(["Apache-2.0"]);
  });

  it("rejects a malformed expression", () => {
    expect(() => parseSpdx("MIT OR")).toThrow();
    expect(() => parseSpdx("(MIT")).toThrow();
  });
});

describe("selectLicenceFiles", () => {
  const f = (names) => names.map((n) => `/pkg/${n}`);

  it("picks the file for the chosen licence, and every NOTICE", () => {
    const files = f(["LICENSE-APACHE", "LICENSE-MIT", "NOTICE"]);
    expect(selectLicenceFiles(files, ["MIT"])).toEqual(f(["LICENSE-MIT", "NOTICE"]));
  });

  it("uses a generically named file when none names the licence", () => {
    expect(selectLicenceFiles(f(["COPYING", "LICENSE"]), ["MIT"])).toEqual(f(["COPYING", "LICENSE"]));
  });

  it("falls back to every file rather than none", () => {
    expect(selectLicenceFiles(f(["LICENSE-APACHE"]), ["MIT"])).toEqual(f(["LICENSE-APACHE"]));
  });
});

describe("repoUrl", () => {
  it("turns repository fields into links", () => {
    expect(repoUrl("git+https://github.com/a/b.git")).toBe("https://github.com/a/b");
    expect(repoUrl("git://github.com/a/b")).toBe("https://github.com/a/b");
    expect(repoUrl("github:a/b")).toBe("https://github.com/a/b");
    expect(repoUrl("a/b")).toBe("https://github.com/a/b");
    expect(repoUrl("git@github.com:a/b.git")).toBe("https://github.com/a/b");
  });
});

describe("packages on disk", () => {
  const dirs = [];
  afterEach(() => {
    for (const d of dirs.splice(0)) fs.rmSync(d, { recursive: true, force: true });
  });
  function tmp() {
    const d = fs.mkdtempSync(path.join(os.tmpdir(), "notices-"));
    dirs.push(d);
    return d;
  }

  it("maps bundled module ids to their packages, scoped and nested", () => {
    const root = tmp();
    const nm = path.join(root, "node_modules");
    for (const [dir, name] of [
      ["a", "a"],
      ["@s/b", "@s/b"],
      ["a/node_modules/c", "c"],
    ]) {
      fs.mkdirSync(path.join(nm, dir), { recursive: true });
      fs.writeFileSync(path.join(nm, dir, "package.json"), JSON.stringify({ name, version: "1.0.0", license: "MIT" }));
    }
    const ids = [
      path.join(nm, "a", "index.js"),
      "\0" + path.join(nm, "@s", "b", "dist", "x.js") + "?commonjs-proxy",
      path.join(nm, "a", "node_modules", "c", "c.css"),
      path.join(root, "src", "own.ts"),
    ];
    expect(jsPackagesFromModules(ids).map((p) => p.name)).toEqual(["@s/b", "a", "c"]);
  });

  it("fills in the standard text, with the authors, when a package ships none", () => {
    const dir = tmp();
    const { text, used, problem } = packageNoticeText(REPO, {
      name: "x",
      license: "MIT OR Apache-2.0",
      authors: ["Jane Doe"],
      dir,
    });
    expect(problem).toBeNull();
    expect(used).toEqual(["MIT"]);
    expect(text).toContain("Copyright (c) Jane Doe");
    expect(text).toContain("Permission is hereby granted");
  });

  it("uses a crate's own licence file, with its NOTICE, and names no licence choice", () => {
    const dir = tmp();
    fs.writeFileSync(path.join(dir, "LICENSE.custom"), "Custom terms\n");
    fs.writeFileSync(path.join(dir, "NOTICE"), "Notice text\n");
    const { text, used, problem } = packageNoticeText(REPO, {
      name: "x",
      license: null,
      authors: [],
      dir,
      licenseFile: path.join(dir, "LICENSE.custom"),
    });
    expect(problem).toBeNull();
    expect(used).toEqual([]);
    expect(text).toBe("Custom terms\n\nNotice text\n");
  });

  it("carries the full CEF and Chromium licence texts, not just their names", () => {
    const text = fs.readFileSync(path.join(REPO, "licenses", "bundled", "cef-chromium.txt"), "utf8");
    expect(text).toContain("Marshall A. Greenblatt");
    expect(text).toContain("The Chromium Authors");
    expect(text.match(/Redistributions in binary form must reproduce/g)).toHaveLength(2);
  });

  it("has Chromium's credits for the CEF version Cargo.lock pins", () => {
    const lock = fs.readFileSync(path.join(REPO, "Cargo.lock"), "utf8");
    const version = lock.match(/name = "cef-dll-sys"\r?\nversion = "([^"]+)"/)[1];
    const { html, problems } = chromiumCredits(REPO, [{ name: "cef-dll-sys", version }]);
    expect(problems).toEqual([]);
    expect(html.toString("utf8")).toContain("<title>Credits");
  });

  it("names the missing credits file for another CEF version", () => {
    const { html, problems } = chromiumCredits(REPO, [{ name: "cef-dll-sys", version: "1.0.0+999.0.1" }]);
    expect(html).toBeNull();
    expect(problems[0]).toContain("credits-cef-999.0.1.html.gz");
  });

  it("has Shiki notices for the installed Shiki version", () => {
    const pj = JSON.parse(
      fs.readFileSync(path.join(REPO, "node_modules", "@shikijs", "langs", "package.json"), "utf8")
    );
    const notices = fs.readFileSync(path.join(REPO, "licenses", "shiki", "notices.txt"), "utf8");
    expect(notices.split("\n")[0]).toMatch(new RegExp(`^Shiki ${pj.version.replace(/\./g, "\\.")}:`));
  });

  it("finds bundled Shiki grammars and themes, and reports one with no section", () => {
    const ids = [
      "/x/node_modules/@shikijs/langs/dist/rust.mjs",
      "C:\\x\\node_modules\\@shikijs\\langs\\dist\\index.mjs",
      "/x/node_modules/@shikijs/themes/dist/github-dark.mjs",
    ];
    const assets = shikiAssetsFromModules(ids);
    expect(assets).toEqual({ grammars: ["rust"], themes: ["github-dark"] });

    const root = tmp();
    fs.mkdirSync(path.join(root, "licenses", "shiki"), { recursive: true });
    const sep = "=".repeat(40);
    fs.writeFileSync(
      path.join(root, "licenses", "shiki", "notices.txt"),
      `Shiki 1.0.0: test\n${sep}\nFiles:   rust.json, toml.json\nSPDX:    MIT\n----\nRust grammar licence\n`
    );
    const { text, problems } = shikiNotices(root, assets);
    expect(text).toContain("Rust grammar licence");
    expect(problems).toEqual([
      'Shiki grammar or theme "github-dark" has no section in ' + path.join("licenses", "shiki", "notices.txt"),
    ]);
  });

  it("parses NOTICE sections by the files they cover", () => {
    const sep = "=".repeat(30);
    const sections = parseNoticeSections(`head\n${sep}\nFiles:   a.json, b.json\nx\n${sep}\nFiles:   c.json\ny\n`);
    expect(sections.map((s) => [...s.names])).toEqual([["a", "b"], ["c"]]);
  });

  it("reports a package it can't cover", () => {
    const { problem } = packageNoticeText(REPO, { name: "x", license: "SSPL-1.0", authors: [], dir: tmp() });
    expect(problem).toMatch(/SSPL-1.0/);
  });
});
