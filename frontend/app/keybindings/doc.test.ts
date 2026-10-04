// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// docs/keybindings.md is generated from the shortcut table; this fails when
// it drifts. Regenerate with UPDATE_KEYBINDINGS_DOC=1.

import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { keybindingsDoc } from "./doc";
import { setUserKeybindings } from "./registry";

const FILE = join(__dirname, "..", "..", "..", "docs", "keybindings.md");

describe("docs/keybindings.md", () => {
    it("matches the shortcut table", () => {
        setUserKeybindings([]);
        const generated = keybindingsDoc();
        if (process.env.UPDATE_KEYBINDINGS_DOC) writeFileSync(FILE, generated);
        expect(readFileSync(FILE, "utf8").replace(/\r\n/g, "\n")).toBe(generated);
    });
});
