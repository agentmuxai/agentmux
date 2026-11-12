// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Vault's stylesheet compiles. Component tests don't load SCSS, so a rule
 *  spliced into the middle of another (unclosed braces) would otherwise reach CI
 *  as a build failure only. */

import { readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import * as sass from "sass";

const dir = __dirname;

describe("the Vault's stylesheets", () => {
    for (const file of readdirSync(dir).filter((f) => f.endsWith(".scss"))) {
        it(`${file} compiles, with its rules at the top level`, () => {
            const css = sass.compile(join(dir, file), { loadPaths: [join(dir, "../../..")] }).css;
            expect(css).toContain(".vault-");
            // A stray nesting would show up as a selector the stylesheet never wrote.
            expect(css).not.toMatch(/\.vault-[\w-]+ \.vault-[\w-]+ \.vault-/);
        });
    }

    it("stacks the pane's lists by the pane's width and a dialog's list by the dialog's", () => {
        const css = sass.compile(join(dir, "vault-view.scss"), { loadPaths: [join(dir, "../../..")] }).css;
        expect(css).toContain("@container vault (max-width: 600px)");
        expect(css).toContain("@container vault (max-width: 700px)");
        expect(css).toContain("@container vault-dialog (max-width: 640px)");
        expect(css).toMatch(/\.vault-dialog-scope\s*\{[^}]*container:\s*vault-dialog/);
    });
});
