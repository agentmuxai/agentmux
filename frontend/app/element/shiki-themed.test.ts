// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Highlighted code picks its colour set by the pane's backdrop inside one, and by the window's theme elsewhere. */
import { join } from "node:path";
import * as sass from "sass";
import { describe, expect, it } from "vitest";

const css = sass.compile(join(__dirname, "shiki-themed.scss")).css.replace(/\s+/g, " ");

describe("shiki-themed.scss", () => {
    it("keeps the window's polarity rule", () => {
        expect(css).toContain('[data-theme-polarity=light] .sh-tok');
    });

    it("lets a pane's backdrop side decide inside the pane, with a rule more specific than the window's", () => {
        expect(css).toContain('[data-backdrop][data-backdrop-scheme=dark] .sh-tok { color: var(--shiki-dark)');
        expect(css).toContain('[data-backdrop][data-backdrop-scheme=light] .sh-tok { color: var(--shiki-light)');
    });
});
