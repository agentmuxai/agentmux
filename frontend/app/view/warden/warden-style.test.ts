// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Warden hides the sections that aren't selected with section-pane.scss's `.bundle-manager-pane.is-hidden`
 *  rule. Nothing else guarantees that stylesheet is loaded (the views' barrels are lazy), so Warden imports it
 *  itself: without it every section shows at once, one under another. */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

describe("Warden's section layout", () => {
    it("loads the stylesheet that hides the sections that aren't selected", () => {
        const view = readFileSync(join(__dirname, "warden-view.tsx"), "utf8");
        expect(view).toMatch(/^import "@\/app\/view\/section-pane\/section-pane\.scss";$/m);
        const rules = readFileSync(join(__dirname, "../section-pane/section-pane.scss"), "utf8");
        expect(rules).toMatch(/\.bundle-manager-pane[\s\S]*&\.is-hidden[\s\S]*display:\s*none/);
    });
});
