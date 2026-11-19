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

    it("measures the pane's lists inside the pane's zoom, not on its outer box", () => {
        const css = sass.compile(join(dir, "vault-view.scss"), { loadPaths: [join(dir, "../../..")] }).css;
        // `.vault-view` carries the zoom; its outer `.vault-container` is 1200px wide at 150% with 800px of room.
        expect(css).toMatch(/\.vault-list\s*\{[^}]*container-name:\s*vault;/);
        expect(css).not.toMatch(/\.vault-container\s*\{[^}]*container/);
    });

    it("shows one line instead of the Vault below 160px of the pane's zoomed width", () => {
        const css = sass.compile(join(dir, "vault-view.scss"), { loadPaths: [join(dir, "../../..")] }).css;
        // The view is a size container inside its zoom, so 160px is 240px of pane at 150%.
        expect(css).toMatch(/\.vault-view\s*\{[^}]*container-name:\s*vault-pane;/);
        const narrow = css.match(/@container vault-pane \(max-width: 159px\)\s*\{([\s\S]*?\n\})\s*\n/);
        expect(narrow).not.toBeNull();
        // Everything but the line goes, and the line (hidden above the limit) comes.
        expect(narrow![1]).toMatch(/\.vault-view > :not\(\.vault-too-narrow\)\s*\{\s*display:\s*none;/);
        expect(narrow![1]).toMatch(/\.vault-too-narrow\s*\{\s*display:\s*block;/);
        expect(css).toMatch(/\.vault-too-narrow\s*\{[^}]*display:\s*none;/);
    });

    it("scrolls the requests when they are taller than the pane, so a card's buttons stay in reach", () => {
        const css = sass.compile(join(dir, "vault-view.scss"), { loadPaths: [join(dir, "../../..")] }).css;
        // The view never scrolls; the requests do, within the pane's height, and are not squeezed by the list.
        expect(css).toMatch(/\.vault-view\s*\{[^}]*overflow:\s*hidden;/);
        const requests = css.match(/\.vault-requests\s*\{([^}]*)\}/);
        expect(requests).not.toBeNull();
        expect(requests![1]).toMatch(/overflow-y:\s*auto;/);
        expect(requests![1]).toMatch(/max-height:\s*100%;/);
        expect(requests![1]).toMatch(/flex:\s*0 0 auto;/);
    });

    it("keeps a chip's words in one box, so a narrow column wraps them at spaces and not inside the name", () => {
        const css = sass.compile(join(dir, "vault-view.scss"), { loadPaths: [join(dir, "../../..")] }).css;
        const text = css.match(/\.vault-grant__text\s*\{([^}]*)\}/);
        expect(text).not.toBeNull();
        expect(text![1]).toMatch(/min-width:\s*0;/);
        expect(text![1]).toMatch(/overflow-wrap:\s*anywhere;/);
    });

    it("clears a stacked cell's own line and padding with a rule that outranks the shared table's", () => {
        const css = sass.compile(join(dir, "vault-view.scss"), { loadPaths: [join(dir, "../../..")] }).css;
        expect(css).toMatch(/\.vault-table tbody td\s*\{[^}]*padding:\s*0;[^}]*border-bottom:\s*0;/);
    });
});
