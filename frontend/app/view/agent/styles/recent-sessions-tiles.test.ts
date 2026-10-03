// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The My Agents tile size is CSS, which jsdom does not compute, so this pins the
// numbers in the stylesheets themselves (SPEC_MY_AGENTS_TILES_AUTH_AND_HISTORY_2026_10_03.md §5.1).

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const read = (name: string) => readFileSync(join(__dirname, name), "utf8");
const picker = read("_picker.scss");
const tiles = read("_recent-sessions.scss");

describe("My Agents tile sizing", () => {
    it("the My Agents grid is twice as wide as before, on its own variable", () => {
        expect(picker).toMatch(/--agent-session-tile-min:\s*480px/);
        expect(tiles).toMatch(/minmax\(min\(var\(--agent-session-tile-min,\s*480px\),\s*100%\),\s*1fr\)/);
    });

    it("the Templates grid keeps its own width", () => {
        expect(picker).toMatch(/--agent-tile-min:\s*240px/);
        expect(tiles).not.toMatch(/var\(--agent-tile-min/);
    });

    it("the summary may use three lines, and the empty-state note stays one", () => {
        const preview = tiles.slice(tiles.indexOf(".agent-recent-sessions-preview {"), tiles.indexOf(".agent-recent-sessions-preview--empty"));
        expect(preview).toMatch(/-webkit-line-clamp:\s*3/);
        expect(preview).not.toMatch(/white-space:\s*nowrap/);
        const empty = tiles.slice(tiles.indexOf(".agent-recent-sessions-preview--empty"), tiles.indexOf(".agent-recent-sessions-line3"));
        expect(empty).toMatch(/-webkit-line-clamp:\s*1/);
    });

    it("the name and account lines are larger", () => {
        expect(tiles).toMatch(/\.agent-recent-sessions-name\s*\{\s*font-size:\s*15px/);
        expect(tiles).toMatch(/\.agent-recent-sessions-account\s*\{\s*font-size:\s*13px/);
    });
});
