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
  jsPackagesFromModules,
  packageNoticeText,
  parseSpdx,
  repoUrl,
  selectLicenceFiles,
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

  it("reports a package it can't cover", () => {
    const { problem } = packageNoticeText(REPO, { name: "x", license: "SSPL-1.0", authors: [], dir: tmp() });
    expect(problem).toMatch(/SSPL-1.0/);
  });
});
