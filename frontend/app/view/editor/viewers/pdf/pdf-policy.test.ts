// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The PDF viewer's safety rules (SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.7).
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { linkAction, MIN_PDFJS_VERSION, SAFE_FLAGS, versionAtLeast } from "./pdf-policy";

describe("the PDF viewer's safety rules", () => {
    it("ships a pdf.js at or past the last script-execution fix", () => {
        const pkg = JSON.parse(readFileSync(join(__dirname, "../../../../../../node_modules/pdfjs-dist/package.json"), "utf8"));
        expect(versionAtLeast(pkg.version, MIN_PDFJS_VERSION), `pdfjs-dist ${pkg.version}`).toBe(true);
    });

    it("opens every document with XFA and eval off", () => {
        expect(SAFE_FLAGS).toEqual({ enableXfa: false, isEvalSupported: false });
    });

    it("sends only http(s) links to a Browser pane, and drops the rest", () => {
        expect(linkAction("https://example.com/a")).toBe("browser");
        expect(linkAction("http://example.com")).toBe("browser");
        expect(linkAction("javascript:alert(1)")).toBe("drop");
        expect(linkAction("file:///C:/Windows/system32/calc.exe")).toBe("drop");
        expect(linkAction("data:text/html,<script>1</script>")).toBe("drop");
        expect(linkAction("ms-settings:privacy")).toBe("drop");
        expect(linkAction("not a url")).toBe("drop");
    });

    it("compares versions numerically", () => {
        expect(versionAtLeast("6.4.299", "6.2.108")).toBe(true);
        expect(versionAtLeast("6.2.108", "6.2.108")).toBe(true);
        expect(versionAtLeast("6.10.0", "6.2.108")).toBe(true);
        expect(versionAtLeast("5.6.83", "6.2.108")).toBe(false);
    });
});
