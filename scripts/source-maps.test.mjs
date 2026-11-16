// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { SourceMapGenerator } from "source-map-js";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { checkMaps, resolveStack } from "./source-maps.mjs";

let dir;
beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), "source-maps-test-"));
  fs.mkdirSync(path.join(dir, "assets"));
});
afterEach(() => fs.rmSync(dir, { recursive: true, force: true }));

/** A chunk `name.js` whose first column maps to `source:line`, named `fn`; written with or without its map. */
function chunk(name, { source = "../../frontend/app/a.ts", map = true, comment = true } = {}) {
  const gen = new SourceMapGenerator({ file: `${name}.js` });
  gen.addMapping({ generated: { line: 1, column: 0 }, original: { line: 39, column: 24 }, source, name: "setFocused" });
  gen.setSourceContent(source, "export const x = 1;");
  fs.writeFileSync(
    path.join(dir, "assets", `${name}.js`),
    `export const t=1;${comment ? `\n//# sourceMappingURL=${name}.js.map` : ""}`
  );
  if (map) fs.writeFileSync(path.join(dir, "assets", `${name}.js.map`), gen.toString());
}

describe("checkMaps", () => {
  it("passes a build whose chunks each have their map, and ignores a chunk that names none", () => {
    chunk("a-1");
    chunk("helper", { map: false, comment: false });
    expect(checkMaps(dir)).toEqual([]);
  });

  it("names a chunk whose map is not there", () => {
    chunk("a-1", { map: false });
    expect(checkMaps(dir).join("\n")).toContain("a-1.js: names a-1.js.map, which is not there");
  });

  it("refuses a map that holds a build machine's absolute path", () => {
    chunk("a-1", { source: "C:/Users/runner/work/frontend/app/a.ts" });
    chunk("a-2", { source: "/home/runner/work/frontend/app/a.ts" });
    const problems = checkMaps(dir).join("\n");
    expect(problems).toContain("a-1.js.map has an absolute source path");
    expect(problems).toContain("a-2.js.map has an absolute source path");
  });

  it("accepts the map of a chunk that is only an inlined asset: no sources, no mappings", () => {
    chunk("a-1");
    fs.writeFileSync(path.join(dir, "assets", "logo.js"), "var e=`<svg/>`;\n//# sourceMappingURL=logo.js.map");
    fs.writeFileSync(
      path.join(dir, "assets", "logo.js.map"),
      '{"version":3,"file":"logo.js","names":[],"sources":[],"mappings":""}'
    );
    expect(checkMaps(dir)).toEqual([]);
  });

  it("fails a build with no maps at all", () => {
    chunk("a-1", { map: false, comment: false });
    expect(checkMaps(dir)).toEqual(["no chunk names a source map: the build has none"]);
  });
});

describe("resolveStack", () => {
  it("rewrites a frame in a chunk that has a map, whichever of its two shapes the frame has", () => {
    chunk("window-focus-abc");
    const out = resolveStack(
      dir,
      [
        "Error: Stale read from <Show>.",
        "    at Ve (http://127.0.0.1:51691/assets/solid.js:1:13234)",
        "    at o (http://127.0.0.1:51691/assets/window-focus-abc.js:1:1)",
        "    at http://127.0.0.1:51691/assets/window-focus-abc.js:1:1",
      ].join("\n")
    );
    const lines = out.split("\n");
    expect(lines[0]).toBe("Error: Stale read from <Show>.");
    expect(lines[1]).toContain("[no map for solid.js]");
    expect(lines[2]).toBe("    at setFocused (../../frontend/app/a.ts:39:25)");
    expect(lines[3]).toBe("    at setFocused (../../frontend/app/a.ts:39:25)");
  });

  it("reads maps from the directory itself too, as an unpacked maps archive lays them out", () => {
    chunk("c-1");
    fs.renameSync(path.join(dir, "assets", "c-1.js.map"), path.join(dir, "c-1.js.map"));
    expect(resolveStack(dir, "    at f (http://x/assets/c-1.js:1:1)")).toContain("a.ts:39:25");
  });
});
