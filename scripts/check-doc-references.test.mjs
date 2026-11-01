// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for check-doc-references.mjs and lib/doc-facts.mjs: what a doc claim is, what
// the code must hold for it to be true, and how the baseline moves.

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import {
    checkDoc,
    codeIndex,
    codeSpans,
    checkDocs,
    compareFindings,
    docFiles,
    formatBaseline,
    isLivingDoc,
    lookupName,
    parseAllow,
    parseBaseline,
    parseDeny,
    pathClaim,
    proseLines,
} from "./check-doc-references.mjs";
import { checkFacts, rustEnumVariants, routeCommandArms, section, tableRows } from "./lib/doc-facts.mjs";

const dirs = [];
afterAll(() => dirs.forEach((d) => rmSync(d, { recursive: true, force: true })));

/** A temporary folder holding `files` (path -> text). */
function tree(files) {
    const root = mkdtempSync(join(tmpdir(), "docref-"));
    dirs.push(root);
    for (const [path, text] of Object.entries(files)) {
        mkdirSync(dirname(join(root, path)), { recursive: true });
        writeFileSync(join(root, path), text);
    }
    return root;
}

const CODE = {
    "crates/app/src/lib.rs": "pub fn open_window() {}\npub struct PaneModel;\nconst MAX_PANES: u32 = 4;\n",
    "crates/app/src/other.rs": "pub fn close_window() {}\n",
    "crates/app/src/tests.rs": "fn t() {}\n",
    "frontend/app/model.ts": "export class LayoutModel { moveTab() {} }\n",
    "docs/specs/SPEC_X_2026_01_01.md": "# spec\n",
};

function indexOf(files = CODE) {
    const root = tree(files);
    return codeIndex(root, Object.keys(files));
}

const kinds = (found) => found.map((f) => `${f.kind}:${f.text}`).sort();

describe("reading a doc", () => {
    it("skips front matter and fenced code, and keeps line numbers", () => {
        const text = ["---", "title: x `crates/no/such.rs`", "---", "prose `a`", "```sh", "ls `crates/in/fence.rs`", "```", "after `b`", ""].join("\n");
        expect(proseLines(text).filter((l) => l.text !== "")).toEqual([
            { n: 4, text: "prose `a`" },
            { n: 8, text: "after `b`" },
        ]);
    });

    it("finds inline code spans, also with doubled backticks", () => {
        expect(codeSpans("see `a.rs` and ``b `c` d`` and `e`")).toEqual(["a.rs", "b `c` d", "e"]);
    });
});

describe("pathClaim", () => {
    const index = indexOf();

    it("reads a path from a top-level folder of the code, with a line number or a symbol", () => {
        expect(pathClaim("crates/app/src/lib.rs", index)).toEqual({ path: "crates/app/src/lib.rs", line: false, symbol: null });
        expect(pathClaim("crates/app/src/lib.rs:12-40", index)).toMatchObject({ path: "crates/app/src/lib.rs", line: true });
        expect(pathClaim("crates/app/src/lib.rs::open_window", index)).toMatchObject({ path: "crates/app/src/lib.rs", symbol: "open_window" });
        expect(pathClaim("specs/SPEC_X.md", index)).toMatchObject({ path: "specs/SPEC_X.md" });
    });

    it("leaves what is not a path into the code", () => {
        for (const span of ["lib.rs", "foo/bar", "crates/*/src", "docs/<name>/x.md", "frontend/{a,b}", "$HOME/crates/x", "two words/here"]) {
            expect(pathClaim(span, index), span).toBeNull();
        }
    });
});

describe("lookupName", () => {
    it("takes names specific enough to look up, and not plain words", () => {
        expect(lookupName("open_window")).toBe("open_window");
        expect(lookupName("PaneModel")).toBe("PaneModel");
        expect(lookupName("Type::member")).toBe("member");
        expect(lookupName("run()")).toBe("run");
        expect(lookupName("MAX_PANES")).toBe("MAX_PANES");
        for (const w of ["pane", "Result", "tab", "foo-bar", "a.b", "x y"]) expect(lookupName(w), w).toBeNull();
    });
});

describe("checkDoc", () => {
    const index = indexOf();

    it("passes a doc whose paths and names are in the code", () => {
        const doc = "See `crates/app/src/lib.rs` for `open_window` and `PaneModel`; `LayoutModel` is in `frontend/app/model.ts`.\n";
        expect(checkDoc(doc, index)).toEqual([]);
    });

    it("flags a path that is not there, with a hint for specs/", () => {
        const found = checkDoc("`crates/app/src/gone.rs` and `specs/SPEC_X_2026_01_01.md` and `docs/specs/`\n", index);
        expect(kinds(found)).toEqual(["missing-path:crates/app/src/gone.rs", "missing-path:specs/SPEC_X_2026_01_01.md"]);
        expect(found.find((f) => f.text.startsWith("specs/")).message).toContain("docs/specs/SPEC_X_2026_01_01.md");
    });

    it("flags a line citation in each of its forms", () => {
        const doc = ["`crates/app/src/lib.rs:12`", "`lib.rs:12-40`", "`model.ts#L12`", "`crates/app/src/lib.rs`, line 437", ""].join("\n");
        expect(checkDoc(doc, index).filter((f) => f.kind === "line-citation")).toHaveLength(4);
    });

    it("looks for a name in the files the line cites, and anywhere in the code when it cites none", () => {
        const doc = [
            "`crates/app/src/lib.rs` has `close_window`.", // only in other.rs
            "`crates/app/src/lib.rs` or `crates/app/src/other.rs` has `close_window`.",
            "`moveTab()` is somewhere, `no_such_thing` is not.",
            "",
        ].join("\n");
        expect(kinds(checkDoc(doc, index, { anySymbol: true }))).toEqual(["missing-symbol:close_window", "missing-symbol:no_such_thing"]);
    });

    it("looks a name up in the whole code only with anySymbol", () => {
        expect(kinds(checkDoc("`no_such_thing` and `CefRenderProcessHandler`\n", index))).toEqual([]);
        // A name on a line that cites a file is looked up in that file, option or not.
        expect(kinds(checkDoc("`crates/app/src/lib.rs` has `no_such_thing`\n", index))).toEqual(["missing-symbol:no_such_thing"]);
    });

    it("does not look names up in the docs themselves", () => {
        const docsOnly = indexOf({ ...CODE, "docs/NOTES.md": "the `only_in_docs_name` thing\n" });
        expect(kinds(checkDoc("`only_in_docs_name`\n", docsOnly, { anySymbol: true }))).toEqual(["missing-symbol:only_in_docs_name"]);
    });

    it("skips what the allow file names", () => {
        const found = checkDoc("`GH_TOKEN` and `crates/app/src/gone.rs`\n", index, { allow: parseAllow("# outside the code\nGH_TOKEN\n") });
        expect(kinds(found)).toEqual(["missing-path:crates/app/src/gone.rs"]);
    });

    it("flags a denied term anywhere, also in a code block", () => {
        const deny = parseDeny("# old names\n.tar.gz => the Linux portable build is .tar.xz\nagent.run-command\n");
        const doc = "Get the `.tar.gz`.\n```\nagent.run-command x\n```\n";
        const found = checkDoc(doc, index, { deny });
        expect(kinds(found)).toEqual(["denied-term:.tar.gz", "denied-term:agent.run-command"]);
        const prose = checkDoc("Use agent.run-command or the .tar.gz build.\n", index, { deny });
        expect(kinds(prose)).toEqual(["denied-term:.tar.gz", "denied-term:agent.run-command"]);
        expect(prose[0].message).toContain("the Linux portable build is .tar.xz");
        expect(prose[1].message).toContain("see the code");
    });
});

describe("file tables", () => {
    const index = indexOf();
    const table = (rows) => ["## The files in `crates/app/src/`", "", "| File | What |", "|---|---|", ...rows.map((r) => `| \`${r}\` | x |`), ""].join("\n");

    it("passes a table that lists every file that is not a test", () => {
        expect(checkDoc(table(["lib.rs", "other.rs"]), index)).toEqual([]);
    });

    it("flags a file the table lacks and a row that is not a file there", () => {
        expect(kinds(checkDoc(table(["lib.rs", "ghost.rs"]), index))).toEqual([
            "table-missing-file:crates/app/src/other.rs",
            "table-unknown-file:crates/app/src/ghost.rs",
        ]);
    });

    it("expects config and asset files in the table, not only source files", () => {
        const withConfig = indexOf({ ...CODE, "crates/app/src/Cargo.toml": "[package]\n", "crates/app/src/seed.json": "{}\n", "crates/app/src/fixtures/a.json": "{}\n" });
        expect(kinds(checkDoc(table(["lib.rs", "other.rs"]), withConfig))).toEqual([
            "table-missing-file:crates/app/src/Cargo.toml",
            "table-missing-file:crates/app/src/seed.json",
        ]);
        expect(checkDoc(table(["lib.rs", "other.rs", "Cargo.toml", "seed.json"]), withConfig)).toEqual([]);
    });

    it("takes a row written as the full repo path as it is", () => {
        expect(checkDoc(table(["crates/app/src/lib.rs", "crates/app/src/other.rs"]), index)).toEqual([]);
        expect(kinds(checkDoc(table(["crates/app/src/lib.rs", "crates/app/src/other.rs", "crates/app/src/ghost.rs"]), index))).toEqual([
            "missing-path:crates/app/src/ghost.rs",
            "table-unknown-file:crates/app/src/ghost.rs",
        ]);
    });
});

describe("docs given as an absolute path", () => {
    it("are found and read, not joined onto the working directory", () => {
        const docs = tree({ "guide/a.md": "See `crates/app/src/ghost.rs`.\n", "guide/b.txt": "x\n" });
        const found = docFiles(process.cwd(), [join(docs, "guide")]);
        expect(found).toHaveLength(1);
        const now = checkDocs(found, process.cwd(), indexOf());
        expect([...now.values()].map((f) => `${f.kind}:${f.text}`)).toEqual(["missing-path:crates/app/src/ghost.rs"]);
    });
});

describe("the baseline", () => {
    it("round-trips and sorts", () => {
        const text = formatBaseline(new Set(["b.md::missing-path::x", "a.md::line-citation::y"]), "scripts/x.baseline");
        expect([...parseBaseline(text)]).toEqual(["a.md::line-citation::y", "b.md::missing-path::x"]);
    });

    it("fails a new finding anywhere, and a fixed one only in a doc the change touches", () => {
        const now = new Map([
            ["a.md::missing-path::new", {}],
            ["a.md::missing-path::old", {}],
        ]);
        const baseline = new Set(["a.md::missing-path::old", "b.md::missing-path::fixed", "c.md::missing-path::fixed"]);
        expect(compareFindings(now, baseline)).toEqual({
            added: ["a.md::missing-path::new"],
            fixed: ["b.md::missing-path::fixed", "c.md::missing-path::fixed"],
        });
        expect(compareFindings(now, baseline, new Set(["b.md"]))).toEqual({
            added: ["a.md::missing-path::new"],
            fixed: ["b.md::missing-path::fixed"],
        });
    });

    it("counts a dated record as no living doc", () => {
        expect(isLivingDoc("docs/MUXLOG.md")).toBe(true);
        expect(isLivingDoc("docs/specs/SPEC_X.md")).toBe(false);
        expect(isLivingDoc("docs/architecture/ARCH_2026_06_13.md")).toBe(false);
        expect(isLivingDoc("docs/reports/R.md")).toBe(false);
    });
});

describe("facts", () => {
    const code = {
        "crates/srv/src/migrations/m0000_a.rs": "",
        "crates/srv/src/migrations/m0001_b.rs": "",
        "crates/srv/src/backend/storage/wan_identity.rs": "const SCHEMA_VERSION: i64 = 5;\n",
        "crates/cef/src/reducer/mod.rs": "pub enum HostCommand {\n    /// one\n    A { x: u8 },\n    B(u8),\n    C,\n}\n",
        "crates/common/src/ipc.rs": "pub enum HwndDriftKind {\n    First,\n    Second,\n}\n",
        "crates/cef/src/ipc.rs": [
            "async fn route_command(cmd: &str) {",
            "    match cmd {",
            '        "get_platform" => ok(),',
            '        "a_cmd" | "b_cmd" => ok(),',
            '        "download_file" => stub(),',
            "        _ => err(),",
            "    }",
            "}",
            "",
        ].join("\n"),
        "crates/cef/src/commands/stubs.rs": 'pub fn is_stub_command(cmd: &str) -> bool {\n    matches!(cmd, "download_file")\n}\n',
        "crates/mcp/src/tool_schemas.rs": '[\n  {\n  "name": "Pane",\n    "properties": { "name": { "type": "string" } }\n  },\n  {\n  "name": "UI"\n  }\n]\n',
        "frontend/app/element/modal-layer.ts": 'type A = { kind: "alpha-one" } | { kind: "bundle-import-select" } | { kind: "bundle-import-confirm" };\n',
        "crates/srv/src/backend/pane_env.rs": 'pub const PANE_ENV_KEEP: &[&str] = &[\n    "AGENTMUX_A",\n    "AGENTMUX_B",\n];\n',
    };

    it("reads the variants of an enum and the arms of route_command", () => {
        expect(rustEnumVariants(code["crates/cef/src/reducer/mod.rs"], "HostCommand")).toEqual(["A", "B", "C"]);
        expect([...routeCommandArms(code["crates/cef/src/ipc.rs"])].sort()).toEqual(["a_cmd", "b_cmd", "download_file", "get_platform"]);
    });

    it("reads sections and tables", () => {
        const text = "## One\n\n| A | B |\n|---|---|\n| `x` | y |\n\n## Two\n\n| A |\n|---|\n| `z` |\n";
        const rows = tableRows(section(text, /^## One/));
        expect(rows.map((r) => r.cells[0])).toEqual(["`x`"]);
        expect(section(text, /^## Missing/)).toBeNull();
    });

    const docs = (changes = {}) => ({
        "internals/data-structure.md": "Migrations `m0000`–`m0001`.\n| `wan.db` | x | y | — (own counter, 5) | z |\n",
        "internals/reducer-stack.md": "its own `HostCommand` (3 variants), host-internal\n",
        "internals/wrr.md": "## The drift kinds\n\n| Kind | What |\n|---|---|\n| `First` | a |\n| `Second` | b |\n",
        "internals/muxapi.md": "## MCP tools (the agent-facing slice)\n\n| Area | Tools |\n|---|---|\n| Panes | [`Pane`](#pane), [`UI`](#ui) |\n\n## Next\n",
        "internals/modal-system.md": "The request kinds today are `alpha-one` and the two `bundle-import-*` steps.\n",
        "internals/env-vars.md": "### The pane allow-list (`PANE_ENV_KEEP`)\n\n| Group | Variables |\n|---|---|\n| g | `AGENTMUX_A`, `AGENTMUX_B` |\n",
        "internals/ipc-catalog.md":
            "### Command Catalog\n\n| Command | Args |\n|---|---|\n| `get_platform` | — |\n\n#### Other commands\n\n| Group | Commands |\n|---|---|\n| g | `a_cmd`, `b_cmd` |\n\n## 4. Channel B\n",
        ...changes,
    });

    const run = (pages) => {
        const root = tree(code);
        const folder = tree(pages);
        return checkFacts(folder, root, Object.keys(code)).map((f) => `${f.page}: ${f.message}`);
    };

    it("finds nothing when the pages match the code", () => {
        expect(run(docs())).toEqual([]);
    });

    it("finds each number, count and list that differs", () => {
        const got = run(
            docs({
                "internals/data-structure.md": "Migrations `m0000`–`m0009`.\n| `wan.db` | x | y | — (own counter, 6) | z |\n",
                "internals/reducer-stack.md": "its own `HostCommand` (4 variants), host-internal\n",
                "internals/wrr.md": "## The drift kinds\n\n| Kind | What |\n|---|---|\n| `First` | a |\n| `Third` | b |\n",
                "internals/muxapi.md": "## MCP tools (the agent-facing slice)\n\n| Area | Tools |\n|---|---|\n| Panes | [`Pane`](#pane) |\n",
                "internals/modal-system.md": "The request kinds today are `alpha-one` and `beta`.\n",
                "internals/env-vars.md": "### The pane allow-list (`PANE_ENV_KEEP`)\n\n| Group | Variables |\n|---|---|\n| g | `AGENTMUX_A`, `AGENTMUX_C` |\n",
                "internals/ipc-catalog.md":
                    "### Command Catalog\n\n| Command | Args |\n|---|---|\n| `get_platform` | — |\n| `gone_cmd` | — |\n\n#### Other commands\n\n| Group | Commands |\n|---|---|\n| g | `a_cmd` |\n\n## 4. Channel B\n",
            })
        );
        expect(got).toEqual(
            expect.arrayContaining([
                expect.stringContaining("migration range: the page says 9, the highest"),
                expect.stringContaining("wan.db schema counter: the page says 6"),
                expect.stringContaining("HostCommand variants: the page says 4"),
                expect.stringContaining("`Third` is documented but is not a variant of HwndDriftKind"),
                expect.stringContaining("`Second` is a variant of HwndDriftKind but the page does not list it"),
                expect.stringContaining("`UI` is a tool in tool_schemas.rs but the page does not list it"),
                expect.stringContaining("`beta` is documented but is not a kind in modal-layer.ts"),
                expect.stringContaining("`bundle-import-select` is a kind in modal-layer.ts but the page does not list it"),
                expect.stringContaining("`AGENTMUX_C` is documented but is not on PANE_ENV_KEEP"),
                expect.stringContaining("`AGENTMUX_B` is on PANE_ENV_KEEP but the page does not list it"),
                expect.stringContaining("`gone_cmd` is documented but is not a command of route_command"),
                expect.stringContaining("`b_cmd` is a command of route_command but the page does not list it"),
            ])
        );
        // The stub `download_file` is neither documented nor expected.
        expect(got.join("\n")).not.toContain("download_file");
    });

    it("skips a page the docs folder does not have", () => {
        expect(run({ "other.md": "x\n" })).toEqual([]);
    });
});
