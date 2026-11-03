// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-no-transition-all.sh, check-muxbus-credential-store.sh, check-menu-positioning.sh,
// check-scrollbar-cursor.sh and check-ligature-fonts.sh against a made-up repo.

import { spawnSync } from "node:child_process";
import { afterEach, describe, expect, it } from "vitest";
import { HAS_BASH, makeFixture } from "./lib/check-fixture.mjs";

const HAS_PERL = HAS_BASH && spawnSync("perl", ["-e", "1"]).status === 0;
const d = HAS_BASH ? describe : describe.skip;

let fx;
afterEach(() => fx?.remove());

function run(gate, files) {
    fx = makeFixture([gate], files);
    return fx.run(gate);
}

d("check-no-transition-all.sh", () => {
    const GATE = "check-no-transition-all.sh";

    it("passes on listed properties", () => {
        const r = run(GATE, { "frontend/a.scss": ".a { transition: opacity 0.2s, transform 0.2s; }\n" });
        expect(r.status).toBe(0);
    });

    it.each([
        ["frontend/a.scss", ".a { transition: all 0.2s; }\n"],
        ["frontend/a.css", ".a {\n  transition:all .2s ease;\n}\n"],
        ["frontend/a.scss", ".a { transition-property: color, all; }\n"],
        ["frontend/a.tsx", 'const s = { transition: "all 0.2s" };\n'],
        ["frontend/a.tsx", 'const c = "transition-all duration-200";\n'],
    ])("fails on %s: %s", (file, text) => {
        const r = run(GATE, { [file]: text });
        expect(r.status).toBe(1);
        expect(r.out).toContain("`transition: all` found");
    });

    it("ignores a comment and a test file", () => {
        const r = run(GATE, {
            "frontend/a.scss": "// transition: all 0.2s;\n.a { color: red; }\n",
            "frontend/a.test.tsx": 'expect(s.transition).toBe("all 1s");\nconst x = { transition: "all" };\n',
        });
        expect(r.status).toBe(0);
    });

    it("does not mistake a longer property name for transition", () => {
        const r = run(GATE, { "frontend/a.scss": ".a { will-transition: all; }\n.b { transition: allow-discrete 1s; }\n" });
        expect(r.status).toBe(0);
    });
});

d("check-muxbus-credential-store.sh", () => {
    const GATE = "check-muxbus-credential-store.sh";

    it("passes when the lookup uses id_store", () => {
        const r = run(GATE, { "crates/srv/src/a.rs": "fn f() { state.id_store.muxbus_load(); }\n" });
        expect(r.status).toBe(0);
    });

    it.each(["muxbus_load", "muxbus_save", "muxbus_clear", "load_valid_token", "ensure_agent_credential", "relay_token"])("fails on state.mstore.%s", (fn) => {
        const r = run(GATE, { "crates/srv/src/a.rs": `fn f() { state.mstore.${fn}(x); }\n` });
        expect(r.status).toBe(1);
        expect(r.out).toContain("crates/srv/src/a.rs");
    });

    it("leaves other calls on state.mstore alone", () => {
        const r = run(GATE, { "crates/srv/src/a.rs": "fn f() { state.mstore.get_block(x); }\n" });
        expect(r.status).toBe(0);
    });
});

d("check-menu-positioning.sh", () => {
    const GATE = "check-menu-positioning.sh";

    it("passes with computePosition( only in the sanctioned files", () => {
        const r = run(GATE, {
            "frontend/app/util/menu-position.ts": "computePosition(a, b);\n",
            "frontend/app/element/tooltip.tsx": "computePosition(a, b);\n",
            "frontend/app/view/x.tsx": "useMenuPosition();\n",
        });
        expect(r.status).toBe(0);
    });

    it("fails on a stray call and shows the line", () => {
        const r = run(GATE, { "frontend/app/view/x.tsx": "const p = await computePosition(a, b);\n" });
        expect(r.status).toBe(1);
        expect(r.out).toContain("- frontend/app/view/x.tsx");
        expect(r.out).toContain("1:const p = await computePosition(a, b);");
    });

    it("looks at .ts and .tsx only", () => {
        const r = run(GATE, { "frontend/app/view/x.md": "computePosition(a)\n" });
        expect(r.status).toBe(0);
    });
});

d("check-scrollbar-cursor.sh", () => {
    const GATE = "check-scrollbar-cursor.sh";

    it("passes when there is no scrollbar selector", () => {
        const r = run(GATE, { "frontend/a.scss": ".a { cursor: pointer; }\n" });
        expect(r.status).toBe(0);
        expect(r.out).toContain("nothing to check");
    });

    it("passes on a scrollbar with the default arrow", () => {
        const r = run(GATE, { "frontend/a.scss": "::-webkit-scrollbar { width: 8px; cursor: var(--cursor-default); }\n" });
        expect(r.status).toBe(0);
    });

    it.each(["pointer", "text", "var(--cursor-interactive)"])("fails on cursor: %s in a scrollbar rule", (value) => {
        const r = run(GATE, { "frontend/a.scss": `::-webkit-scrollbar-thumb {\n  background: red;\n  cursor: ${value};\n}\n` });
        expect(r.status).toBe(1);
        expect(r.out).toContain("frontend/a.scss:3");
    });

    it("fails in a rule nested under an os-scrollbar selector", () => {
        const r = run(GATE, { "frontend/a.scss": ".os-scrollbar {\n  &:hover {\n    cursor: pointer;\n  }\n}\n" });
        expect(r.status).toBe(1);
    });

    it("does not blame a pointer cursor outside the scrollbar rule", () => {
        const r = run(GATE, { "frontend/a.scss": "::-webkit-scrollbar { width: 8px; }\n.button { cursor: pointer; }\n" });
        expect(r.status).toBe(0);
    });
});

const p = HAS_PERL ? describe : describe.skip;

p("check-ligature-fonts.sh", () => {
    const GATE = "check-ligature-fonts.sh";

    it("passes on a font stack with no ligature font", () => {
        const r = run(GATE, { "frontend/a.scss": "body { font-family: Hack, monospace; }\n" });
        expect(r.status).toBe(0);
    });

    it("fails on a ligature font named in a font stack", () => {
        const r = run(GATE, { "frontend/a.scss": 'body { font-family: "Fira Code", monospace; }\n' });
        expect(r.status).toBe(1);
        expect(r.out).toContain("named in a CSS font stack");
        expect(r.out).toContain("frontend/a.scss:1");
    });

    it("ignores a name in a block comment or a line comment", () => {
        const r = run(GATE, { "frontend/a.scss": "/* JetBrains Mono was removed */\n// Iosevka too\nbody { color: red; }\n" });
        expect(r.status).toBe(0);
    });

    it("fails on a bundled ligature font file", () => {
        const r = run(GATE, { "public/fonts/JetBrainsMono-Regular.woff2": "x" });
        expect(r.status).toBe(1);
        expect(r.out).toContain("bundled in public/fonts/");
    });
});
