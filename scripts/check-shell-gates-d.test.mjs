// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-rpc-bindings.sh, check-toolchain.sh, check-spec-citations.sh and check-doc-status.sh against a made-up repo.

import { spawnSync } from "node:child_process";
import { chmodSync, copyFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { HAS_BASH, makeFixture } from "./lib/check-fixture.mjs";

const d = HAS_BASH ? describe : describe.skip;

let fx;
afterEach(() => fx?.remove());

/** A git fixture on `main`, then a `feature` branch carrying `change(fx)`. */
function branched(gate, files, change) {
    fx = makeFixture([gate], files).git();
    fx.gitRun("branch", "-M", "main");
    fx.gitRun("checkout", "-q", "-b", "feature");
    change(fx);
    fx.gitRun("add", "-A");
    fx.gitRun("commit", "-q", "--allow-empty", "-m", "change");
    return fx;
}

d("check-rpc-bindings.sh", () => {
    const GATE = "check-rpc-bindings.sh";
    const TYPE = "frontend/types/rpc/Thing.ts";

    it("passes with --check-only when the generated folder is clean", () => {
        fx = makeFixture([GATE], { [TYPE]: "export type Thing = {};\n" }).git();
        const r = fx.run(GATE, ["--check-only"]);
        expect(r.status).toBe(0);
        expect(r.out).toContain("ok (1 generated type(s) current)");
    });

    it("fails with --check-only when regenerating changed a committed binding", () => {
        fx = makeFixture([GATE], { [TYPE]: "export type Thing = {};\n" }).git();
        fx.write(TYPE, "export type Thing = { id: string };\n");
        const r = fx.run(GATE, ["--check-only"]);
        expect(r.status).toBe(1);
        expect(r.out).toContain("generated RPC bindings are STALE");
        expect(r.out).toContain(TYPE);
    });

    it("fails with --check-only on a binding that was generated and never committed", () => {
        fx = makeFixture([GATE], { [TYPE]: "export type Thing = {};\n" }).git();
        fx.write("frontend/types/rpc/New.ts", "export type New = {};\n");
        expect(fx.run(GATE, ["--check-only"]).status).toBe(1);
    });

    it("refuses to regenerate over uncommitted edits, before it runs cargo", () => {
        fx = makeFixture([GATE], { [TYPE]: "export type Thing = {};\n" }).git();
        fx.write(TYPE, "export type Thing = { edited: true };\n");
        const r = fx.run(GATE);
        expect(r.status).toBe(1);
        expect(r.out).toContain("has uncommitted changes before regenerating");
    });

    it("prints its usage and exits 2 on an unknown argument", () => {
        fx = makeFixture([GATE], {});
        const r = fx.run(GATE, ["--nope"]);
        expect(r.status).toBe(2);
        expect(r.out).toContain("usage:");
    });
});

// The gate runs with a PATH of only its fakes and a few coreutils copied from the host, which a Windows shell cannot do.
const t = HAS_BASH && process.platform !== "win32" ? describe : describe.skip;

t("check-toolchain.sh", () => {
    const GATE = "check-toolchain.sh";

    function realTool(name) {
        const r = spawnSync("bash", ["-c", `command -v ${name}`], { encoding: "utf8" });
        return r.stdout.trim();
    }

    /**
     * Run the gate on a `Linux` whose PATH holds only `tools` ({ name: "version output" }) and the few coreutils it
     * calls, so each tool is present, old or missing on purpose.
     */
    function run(tools) {
        fx = makeFixture([GATE], {});
        fx.write("bin/uname", "#!/bin/sh\necho Linux\n");
        chmodSync(join(fx.root, "bin/uname"), 0o755);
        for (const name of ["cut", "grep", "head"]) copyFileSync(realTool(name), join(fx.root, "bin", name));
        for (const [name, version] of Object.entries(tools)) {
            fx.write(`bin/${name}`, `#!/bin/sh\necho "${version}"\n`);
            chmodSync(join(fx.root, `bin/${name}`), 0o755);
        }
        return fx.run(GATE, [], { env: { PATH: join(fx.root, "bin") } });
    }

    const GOOD = { git: "git version 2.40.0", cargo: "cargo 1.99.1 (abc 2026-01-01)", cmake: "cmake version 3.28.1", ninja: "1.11.1", node: "v24.21.0" };

    it("passes when every tool is present and recent enough", () => {
        const r = run(GOOD);
        expect(r.status).toBe(0);
        expect(r.out).toContain("All required tools present and up to date.");
    });

    it("fails and says what to install when a tool is missing", () => {
        const { ninja, ...rest } = GOOD;
        const r = run(rest);
        expect(r.status).toBe(1);
        expect(r.out).toContain("MISSING  ninja");
    });

    it("fails on a tool below its minimum, comparing each version part as a number", () => {
        const r = run({ ...GOOD, cmake: "cmake version 3.9.0" });
        expect(r.status).toBe(1);
        expect(r.out).toContain("TOO OLD  cmake — found 3.9.0, need >=3.20");
        const ok = run({ ...GOOD, cmake: "cmake version 3.100.0" });
        expect(ok.status).toBe(0);
    });

    it("lets a tool through when its version output cannot be parsed", () => {
        const r = run({ ...GOOD, git: "git" });
        expect(r.status).toBe(0);
    });

    it.each(["v24.10.9", "v22.0.0", "v18.20.1"])("fails on node %s", (version) => {
        const r = run({ ...GOOD, node: version });
        expect(r.status).toBe(1);
        expect(r.out).toContain("WRONG VERSION  node.js");
    });

    it.each(["v24.11.0", "v25.0.0"])("accepts node %s", (version) => {
        expect(run({ ...GOOD, node: version }).status).toBe(0);
    });

    it("fails when node is missing", () => {
        const { node, ...rest } = GOOD;
        const r = run(rest);
        expect(r.status).toBe(1);
        expect(r.out).toContain("MISSING  node.js");
    });
});

d("check-spec-citations.sh", () => {
    const GATE = "check-spec-citations.sh";

    function run(files, ...args) {
        fx = makeFixture([GATE], files);
        return fx.run(GATE, args);
    }

    it("passes when every cited doc exists", () => {
        const r = run({ "docs/specs/S.md": "# S\n", "src/a.rs": "// see docs/specs/S.md\n" }, "src/a.rs");
        expect(r.status).toBe(0);
        expect(r.out).toContain("ok (1 file(s) checked)");
    });

    it("fails on a citation of a doc that does not exist, and names it", () => {
        const r = run({ "src/a.rs": "// see docs/specs/GONE.md for why\n" }, "src/a.rs");
        expect(r.status).toBe(1);
        expect(r.out).toContain("FAIL src/a.rs");
        expect(r.out).toContain("Cites a doc that does not exist: docs/specs/GONE.md");
    });

    it("skips globs, docs/en/, tests, fixtures and binary files", () => {
        const r = run(
            {
                "src/a.rs": "// docs/specs/*.md and docs/en/guide.md\n",
                "src/a.test.rs": "// docs/specs/GONE.md\n",
                "fixtures/x.md": "docs/specs/GONE.md\n",
                "logo.svg": "docs/specs/GONE.md\n",
            },
            "src/a.rs",
            "src/a.test.rs",
            "fixtures/x.md",
            "logo.svg",
        );
        expect(r.status).toBe(0);
        expect(r.out).toContain("(1 file(s) checked)");
    });

    it("does not read a path inside a longer path as a citation", () => {
        const r = run({ "src/a.rs": "// ../vendor/docs/specs/GONE.md\n" }, "src/a.rs");
        expect(r.status).toBe(0);
    });

    it("checks the files the branch changed against its base", () => {
        branched(GATE, { "docs/specs/S.md": "# S\n", "src/old.rs": "// docs/specs/GONE.md\n" }, (f) => f.write("src/new.rs", "// docs/specs/GONE2.md\n"));
        const r = fx.run(GATE);
        expect(r.status).toBe(1);
        expect(r.out).toContain("FAIL src/new.rs");
        expect(r.out).not.toContain("src/old.rs");
    });

    it("says nothing changed on a branch with no changes, and skips when it has no base", () => {
        branched(GATE, { "a.txt": "1" }, () => {});
        fx.gitRun("checkout", "-q", "main");
        expect(fx.run(GATE).out).toContain("nothing changed");
        fx.remove();
        fx = makeFixture([GATE], {});
        const r = fx.run(GATE);
        expect(r.status).toBe(0);
        expect(r.out).toContain("cannot resolve base ref");
    });
});

d("check-doc-status.sh", () => {
    const GATE = "check-doc-status.sh";
    const spec = (status, extra = "") => `# S\n\n**Status:** ${status}\n${extra}`;

    function run(files, ...args) {
        fx = makeFixture([GATE], files);
        return fx.run(GATE, args);
    }

    it.each(["draft", "proposed", "active — in #1", "implemented — #2", "living", "historical", "retro", "analysis"])("accepts the Status %s", (status) => {
        const r = run({ "docs/specs/a.md": spec(status) }, "docs/specs/a.md");
        expect(r.status).toBe(0);
        expect(r.out).toContain("ok (1 doc(s) checked)");
    });

    it("fails on a Status outside the closed list", () => {
        const r = run({ "docs/specs/a.md": spec("maybe someday") }, "docs/specs/a.md");
        expect(r.status).toBe(1);
        expect(r.out).toContain("Status must start with one of");
    });

    it("ignores the case of the Status word and punctuation after it", () => {
        const r = run({ "docs/specs/a.md": spec("Implemented, shipped in #5") }, "docs/specs/a.md");
        expect(r.status).toBe(0);
    });

    it("passes an explicit file that has no Status line", () => {
        const r = run({ "docs/specs/a.md": "# Just a heading\n" }, "docs/specs/a.md");
        expect(r.status).toBe(0);
    });

    it("requires a superseded doc to point at a file that exists", () => {
        expect(run({ "docs/specs/a.md": spec("superseded") }, "docs/specs/a.md").out).toContain("no **Superseded-by:** line");
        expect(run({ "docs/specs/a.md": spec("superseded", "**Superseded-by:** soon\n") }, "docs/specs/a.md").out).toContain("names no repository path");
        expect(run({ "docs/specs/a.md": spec("superseded", "**Superseded-by:** gone.md\n") }, "docs/specs/a.md").out).toContain("names no path that exists");
        const ok = run({ "docs/specs/a.md": spec("superseded", "**Superseded-by:** b.md\n"), "docs/specs/b.md": "# B\n" }, "docs/specs/a.md");
        expect(ok.status).toBe(0);
    });

    it("fails when a top-level specs/ folder exists", () => {
        const r = run({ "specs/old.md": "# old\n" }, "specs/old.md");
        expect(r.status).toBe(1);
        expect(r.out).toContain("A top-level specs/ tree exists");
    });

    it("on a branch, fails a new doc with no Status line but not an old one", () => {
        branched(GATE, { "docs/specs/old.md": "# Old, no status\n" }, (f) => {
            f.write("docs/specs/new.md", "# New, no status\n");
            f.write("docs/specs/old.md", "# Old, edited\n");
        });
        const r = fx.run(GATE);
        expect(r.status).toBe(1);
        expect(r.out).toContain("FAIL docs/specs/new.md");
        expect(r.out).not.toContain("docs/specs/old.md");
    });

    it("on a branch, passes a new doc with a valid Status and ignores a change outside docs/", () => {
        branched(GATE, { "a.txt": "1" }, (f) => {
            f.write("docs/specs/new.md", spec("draft"));
            f.write("a.txt", "2");
        });
        const r = fx.run(GATE);
        expect(r.status).toBe(0);
        expect(r.out).toContain("(1 doc(s) checked)");
    });

    it("says no docs changed when the branch touches none", () => {
        branched(GATE, { "a.txt": "1" }, (f) => f.write("a.txt", "2"));
        expect(fx.run(GATE).out).toContain("no docs changed");
    });
});
