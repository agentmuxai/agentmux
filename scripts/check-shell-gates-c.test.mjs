// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-bundled-tools.sh, check-claude-md-references.sh and check-remote-helpers.sh against a made-up repo.

import { chmodSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { HAS_BASH, makeFixture } from "./lib/check-fixture.mjs";

const d = HAS_BASH ? describe : describe.skip;

let fx;
afterEach(() => fx?.remove());

d("check-bundled-tools.sh", () => {
    const GATE = "check-bundled-tools.sh";
    const CONFIG = "crates/srv/src/backend/agent_config.rs";
    const TOOLS = ["agentmux-mcp", "agentmux-bashwrap"];
    const PACKAGERS = ["scripts/package-portable.sh", "scripts/package-macos.sh", "scripts/stage-linux-runtime.sh"];

    const config = TOOLS.map((t) => `    "command": "${t}",\n`).join("");
    const packager = (tools = TOOLS, helpers = true) =>
        tools.map((t) => `cp target/release/${t} "$STAGE/tools/bin/"\n`).join("") +
        (helpers ? 'bash scripts/build-remote-helpers.sh "$STAGE/tools/remote"\n' : "");

    function run(overrides = {}) {
        const files = { [CONFIG]: config, ...Object.fromEntries(PACKAGERS.map((p) => [p, packager()])), ...overrides };
        fx = makeFixture([GATE], files);
        return fx.run(GATE);
    }

    it("passes when every packager copies every tool and builds the remote helpers", () => {
        const r = run();
        expect(r.status).toBe(0);
        expect(r.out).toContain("3 packagers each bundle: agentmux-bashwrap agentmux-mcp");
    });

    it("accepts a Windows .exe copy and a cp-then-symlink", () => {
        const r = run({
            "scripts/package-portable.sh": TOOLS.map((t) => `cp target/release/${t}.exe "$STAGE/tools/bin/"\n`).join("") + packager([], true),
            "scripts/package-macos.sh":
                TOOLS.map((t) => `cp target/release/${t} "$STAGE/lib/"\nln -s ../lib/${t} "$STAGE/tools/bin/${t}"\n`).join("") + packager([], true),
        });
        expect(r.status).toBe(0);
    });

    it("fails when the config names no agentmux- command at all", () => {
        const r = run({ [CONFIG]: "nothing here\n" });
        expect(r.status).toBe(1);
        expect(r.out).toContain("found no");
    });

    it("fails when one packager leaves a tool out", () => {
        const r = run({ "scripts/package-macos.sh": packager(["agentmux-mcp"]) });
        expect(r.status).toBe(1);
        expect(r.out).toContain("scripts/package-macos.sh does not copy agentmux-bashwrap into tools/bin");
    });

    it("fails when a packager does not build the remote helpers", () => {
        const r = run({ "scripts/stage-linux-runtime.sh": packager(TOOLS, false) });
        expect(r.status).toBe(1);
        expect(r.out).toContain("scripts/stage-linux-runtime.sh does not build the remote helpers");
    });

    it("fails when a packager has gone", () => {
        fx = makeFixture([GATE], { [CONFIG]: config, "scripts/package-portable.sh": packager(), "scripts/package-macos.sh": packager() });
        const r = fx.run(GATE);
        expect(r.status).toBe(1);
        expect(r.out).toContain("scripts/stage-linux-runtime.sh no longer exists");
    });
});

d("check-claude-md-references.sh", () => {
    const GATE = "check-claude-md-references.sh";

    function run(files) {
        fx = makeFixture([GATE], files).git();
        return fx.run(GATE);
    }

    it("passes when no document mentions CLAUDE.md", () => {
        expect(run({ "README.md": "Hello.\n" }).status).toBe(0);
    });

    it("fails on a living doc that points at the deleted repo-level CLAUDE.md", () => {
        const r = run({ "docs/guide.md": "See CLAUDE.md for the build steps.\n" });
        expect(r.status).toBe(1);
        expect(r.out).toContain("FAIL docs/guide.md:1");
        expect(r.out).toContain("points at the deleted");
    });

    it("lets a line about a CLAUDE.md that AgentMux generates through", () => {
        const r = run({ "docs/guide.md": "AgentMux writes the generated per-agent CLAUDE.md at launch.\nIt also reads .claude/settings.json next to CLAUDE.md.\n" });
        expect(r.status).toBe(0);
    });

    it("exempts dated records and changelogs", () => {
        const r = run({
            "CHANGELOG.md": "Removed CLAUDE.md.\n",
            "docs/analysis/a.md": "CLAUDE.md was wrong.\n",
            "docs/retro/r-2026-09-01.md": "CLAUDE.md\n",
            "docs/reports/x.md": "CLAUDE.md\n",
        });
        expect(r.status).toBe(0);
    });

    it("does not read a file that is not Markdown", () => {
        expect(run({ "notes.txt": "See CLAUDE.md\n" }).status).toBe(0);
    });
});

d("check-remote-helpers.sh", () => {
    const GATE = "check-remote-helpers.sh";

    const elf = (machine) => {
        const b = Buffer.alloc(20);
        Buffer.from("7f454c46", "hex").copy(b, 0);
        Buffer.from(machine, "hex").copy(b, 18);
        return b;
    };
    const macho = (cpu) => Buffer.concat([Buffer.from("cffaedfe", "hex"), Buffer.from(cpu, "hex"), Buffer.alloc(12)]);
    const BUILDS = {
        "x86_64-unknown-linux-musl": () => elf("3e00"),
        "aarch64-unknown-linux-musl": () => elf("b700"),
        "x86_64-apple-darwin": () => macho("07000001"),
        "aarch64-apple-darwin": () => macho("0c000001"),
    };

    /** A repo where `uname` reports an OS nothing is built for, so each helper is checked for format and not run. */
    function run(overrides = {}) {
        const files = {
            "Cargo.toml": '[workspace.package]\nversion = "1.2.3"\n\n[workspace.dependencies]\nx = "1"\n',
            "crates/remote/src/frame.rs": "pub const PROTOCOL: u32 = 7;\n",
            "bin/uname": "#!/bin/sh\necho Plan9\n",
            ...Object.fromEntries(Object.entries(BUILDS).map(([t, make]) => [`dist/remote/${t}/agentmux-remote`, make()])),
            ...overrides,
        };
        fx = makeFixture([GATE], {});
        for (const [rel, data] of Object.entries(files)) if (data !== null) fx.write(rel, data);
        chmodSync(join(fx.root, "bin/uname"), 0o755);
        return fx.run(GATE, [], { env: { PATH: `${join(fx.root, "bin")}:${process.env.PATH}` } });
    }

    it("passes when all four builds have the right format", () => {
        const r = run();
        expect(r.status).toBe(0);
        expect(r.out).toContain("x86_64-apple-darwin: format ok");
        expect(r.out).toContain("all four builds present and well-formed; 0 ran");
    });

    it("fails when a build is missing", () => {
        const r = run({ "dist/remote/aarch64-apple-darwin/agentmux-remote": null });
        expect(r.status).toBe(1);
        expect(r.out).toContain("missing dist/remote/aarch64-apple-darwin/agentmux-remote");
    });

    it("fails when a build is for the wrong CPU", () => {
        const r = run({ "dist/remote/x86_64-unknown-linux-musl/agentmux-remote": elf("b700") });
        expect(r.status).toBe(1);
        expect(r.out).toContain("is not a x86_64-unknown-linux-musl binary");
    });

    it("fails when a Mach-O build is really the other architecture", () => {
        const r = run({ "dist/remote/aarch64-apple-darwin/agentmux-remote": macho("07000001") });
        expect(r.status).toBe(1);
        expect(r.out).toContain("is not a aarch64-apple-darwin binary");
    });

    it("fails when it cannot read the version or protocol", () => {
        const r = run({ "crates/remote/src/frame.rs": "// no protocol\n" });
        expect(r.status).toBe(1);
        expect(r.out).toContain("could not read the version");
    });

    it("reads the version from [workspace.package] only", () => {
        const r = run({ "Cargo.toml": '[dependencies]\nversion = "9"\n' });
        expect(r.status).toBe(1);
        expect(r.out).toContain("could not read the version");
    });
});
