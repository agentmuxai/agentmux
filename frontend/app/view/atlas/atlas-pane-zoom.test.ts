// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { atlasPaneTab } from "./atlas";

const scss = readFileSync(join(__dirname, "atlas.scss"), "utf8");

/** The declarations of the first rule for `selector` in atlas.scss, comments removed. */
function rule(selector: string): string {
    const start = scss.indexOf(`${selector} {`);
    expect(start, `${selector} in atlas.scss`).toBeGreaterThanOrEqual(0);
    const end = scss.indexOf("\n}", start);
    return scss
        .slice(start, end)
        .split("\n")
        .filter((line) => !line.trim().startsWith("//"))
        .join("\n");
}

describe("Atlas takes the pane zoom", () => {
    it("says it takes part in the pane zoom, and applies the level block.tsx gives it as `--pane-zoom`", () => {
        // The capability is what makes the zoom keys and Ctrl+scroll act on the pane and block.tsx publish the level.
        expect(atlasPaneTab.capabilities?.paneZoom).toBeTruthy();
        expect(rule(".atlas")).toContain("zoom: var(--pane-zoom, 1);");
    });
});
