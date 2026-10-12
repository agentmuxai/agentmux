// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";
import path from "node:path";
import { describe, expect, it } from "vitest";

// The script is run in its own Node process, not imported: this repo's vitest
// config resolves packages with the browser condition, and remark's entity
// decoder then loads its DOM build, which needs `document`.
const MODULE = pathToFileURL(path.join(import.meta.dirname, "link-md-paths.mjs")).href;
function call(fn, ...args) {
  const src = `import * as m from ${JSON.stringify(MODULE)};
const [a, t] = JSON.parse(process.argv[1]);
const tracked = new Set(t);
process.stdout.write(JSON.stringify(m[${JSON.stringify(fn)}](...a, tracked) ?? null));`;
  const out = execFileSync(process.execPath, ["--input-type=module", "-e", src, JSON.stringify([args, [...TRACKED]])], {
    encoding: "utf8",
  });
  return JSON.parse(out);
}
const resolveTarget = (value, mdFile) => call("resolveTarget", value, mdFile);
const linkPaths = (text, mdFile) => call("linkPaths", text, mdFile);

const TRACKED = new Set([
  "README.md",
  "Cargo.toml",
  "crates",
  "crates/srv",
  "crates/srv/src",
  "crates/srv/src/main.rs",
  "docs",
  "docs/specs",
  "docs/specs/SPEC_A.md",
  "docs/specs/SPEC_B.md",
  "scripts",
  "scripts/release.sh",
]);

describe("resolveTarget", () => {
  it("links a repo-rooted path from the root", () => {
    expect(resolveTarget("crates/srv/src/main.rs", "README.md")).toBe("crates/srv/src/main.rs");
  });
  it("writes the link relative to the doc", () => {
    expect(resolveTarget("crates/srv/src/main.rs", "docs/specs/SPEC_A.md")).toBe(
      "../../crates/srv/src/main.rs",
    );
  });
  it("resolves a path relative to the doc when the root has no such file", () => {
    expect(resolveTarget("specs/SPEC_B.md", "docs/README.md")).toBe("specs/SPEC_B.md");
  });
  it("turns :N and :N-M into line anchors", () => {
    expect(resolveTarget("scripts/release.sh:183", "README.md")).toBe("scripts/release.sh#L183");
    expect(resolveTarget("scripts/release.sh:10-20", "README.md")).toBe("scripts/release.sh#L10-L20");
    expect(resolveTarget("scripts/release.sh:10–20", "README.md")).toBe("scripts/release.sh#L10-L20");
  });
  it("adds no line anchor to a Markdown target", () => {
    expect(resolveTarget("docs/specs/SPEC_A.md:12", "README.md")).toBe("docs/specs/SPEC_A.md");
  });
  it("links a folder, with or without a trailing slash", () => {
    expect(resolveTarget("crates/srv/", "README.md")).toBe("crates/srv");
    expect(resolveTarget("crates/", "README.md")).toBe("crates");
  });
  it("links a bare file name only from a root-level doc", () => {
    expect(resolveTarget("Cargo.toml", "README.md")).toBe("Cargo.toml");
    expect(resolveTarget("Cargo.toml", "docs/specs/SPEC_A.md")).toBeNull();
    expect(resolveTarget("crates", "README.md")).toBeNull();
  });
  it("leaves untracked, missing and non-path values alone", () => {
    expect(resolveTarget("crates/srv/src/gone.rs", "README.md")).toBeNull();
    expect(resolveTarget("npm run build", "README.md")).toBeNull();
    expect(resolveTarget("~/.agentmux/agents", "README.md")).toBeNull();
    expect(resolveTarget("https://example.com/a.md", "README.md")).toBeNull();
    expect(resolveTarget("crates/*/src", "README.md")).toBeNull();
  });
});

describe("linkPaths", () => {
  it("wraps the code span in a link and keeps everything else", () => {
    const { text, changes } = linkPaths("See `crates/srv/src/main.rs` and `npm test`.\n", "README.md");
    expect(text).toBe("See [`crates/srv/src/main.rs`](crates/srv/src/main.rs) and `npm test`.\n");
    expect(changes).toHaveLength(1);
  });
  it("links several paths on one line, and paths in tables", () => {
    const md = "| a | b |\n|---|---|\n| `Cargo.toml` | `scripts/release.sh:5` |\n";
    expect(linkPaths(md, "README.md").text).toBe(
      "| a | b |\n|---|---|\n| [`Cargo.toml`](Cargo.toml) | [`scripts/release.sh:5`](scripts/release.sh#L5) |\n",
    );
  });
  it("skips fenced code, existing links and headings", () => {
    const md = [
      "## The `crates/srv` crate",
      "",
      "```",
      "`crates/srv`",
      "```",
      "",
      "Already [`crates/srv`](crates/srv) and [see `Cargo.toml`](Cargo.toml).",
      "",
    ].join("\n");
    const { text, changes } = linkPaths(md, "README.md");
    expect(changes).toHaveLength(0);
    expect(text).toBe(md);
  });
  it("leaves YAML frontmatter alone", () => {
    const md = "---\ndescription: see `Cargo.toml`\n---\n\nBody `Cargo.toml`.\n";
    expect(linkPaths(md, "README.md").text).toBe(
      "---\ndescription: see `Cargo.toml`\n---\n\nBody [`Cargo.toml`](Cargo.toml).\n",
    );
  });
  it("turns a two-space hard break on an edited line into a backslash", () => {
    const md = "**File:** `Cargo.toml`  \nnext line  \n\nend `Cargo.toml`  \n";
    expect(linkPaths(md, "README.md").text).toBe(
      "**File:** [`Cargo.toml`](Cargo.toml)\\\nnext line  \n\nend [`Cargo.toml`](Cargo.toml)\n",
    );
  });
  it("is idempotent", () => {
    const once = linkPaths("`crates/srv/src/main.rs`\n", "README.md").text;
    expect(linkPaths(once, "README.md").changes).toHaveLength(0);
  });
});
