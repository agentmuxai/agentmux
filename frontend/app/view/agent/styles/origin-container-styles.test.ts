// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The origin container's stylesheet compiled: a selector that reads right in the source can compile
 * to one that never matches (`&--folded &__text` inside `.agent-view .agent-origin` became
 * `.agent-origin--folded .agent-view .agent-origin__text`, so a folded body was never clamped).
 */

import { join } from "node:path";
import * as sass from "sass";
import { describe, expect, it } from "vitest";

const css = sass.compile(join(__dirname, "..", "agent-view.scss"), { loadPaths: [join(__dirname, "..", "..", "..", "..")], quietDeps: true, logger: sass.Logger.silent }).css;
const rule = (selector: string): string | undefined => {
    const at = css.indexOf(`${selector} {`);
    return at < 0 ? undefined : css.slice(at, css.indexOf("}", at));
};

describe("origin container styles", () => {
    it("a folded body is clamped to a few lines", () => {
        const body = rule(".agent-view .agent-origin--folded .agent-origin__text");
        expect(body).toMatch(/-webkit-line-clamp:\s*4/);
        expect(body).toMatch(/overflow:\s*hidden/);
    });

    it("no origin selector repeats .agent-view inside itself", () => {
        const repeated = css
            .split("\n")
            .filter((l) => l.includes(".agent-origin") && l.trimEnd().endsWith("{"))
            .flatMap((l) => l.replace(/\s*\{$/, "").split(","))
            .filter((sel) => (sel.match(/\.agent-view\b/g) ?? []).length > 1);
        expect(repeated).toEqual([]);
    });

    it("the jekt variant's body keeps the preview-box cap", () => {
        expect(rule(".agent-view .agent-origin-body")).toMatch(/max-height:/);
    });
});
