// The frontend toolchain runs two TypeScripts side by side, and which one a name
// reaches is decided by how npm links things, so it is pinned here.
//
//   * `tsc` is TypeScript 7 (the native compiler). `@typescript/native` is the
//     `typescript@7` package under another name; its `tsc` bin has to be the one
//     in node_modules/.bin. `@typescript/typescript6` pulls in `@typescript/old`
//     (typescript 6), which ships a `tsc` bin of its own, and npm links the
//     first of two same-named bins in name order: renaming `@typescript/native`
//     to a name that sorts after `@typescript/old` would silently make
//     `npx tsc` (and the CI typecheck) run TypeScript 6 again.
//   * `import "typescript"` is the TypeScript 6 API package (`typescript` is an
//     alias of `@typescript/typescript6` in package.json). TypeScript 7 exports no
//     compiler API from its package root, and typescript-eslint, knip,
//     prettier-plugin-organize-imports, scripts/gen-mux-events.mjs and
//     literal-ratchet.test.ts all read that API.
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { describe, expect, it } from "vitest";

const require = createRequire(import.meta.url);
const root = new URL("..", import.meta.url);

describe("frontend toolchain: TypeScript 7 compiler, TypeScript 6 API", () => {
  it("`tsc` in node_modules/.bin is TypeScript 7", () => {
    const r = spawnSync("npx", ["--no-install", "tsc", "--version"], {
      cwd: root,
      encoding: "utf8",
      shell: true,
    });
    expect(r.stdout.trim()).toMatch(/^Version 7\./);
  });

  it('`import "typescript"` still has the compiler API, at version 6', () => {
    const ts = require("typescript");
    expect(ts.version).toMatch(/^6\./);
    expect(typeof ts.createSourceFile).toBe("function");
  });
});
