// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for sync-lockfile-version.mjs's pure parts — no filesystem.
//
// The regression this guards: the previous inline `node -e` in
// bump-wrapper.sh hardcoded JSON.stringify(lock, null, 2), silently
// reformatting this repo's 4-space package-lock.json to 2-space on every
// release (a ~24,000-line diff carrying zero dependency changes). See the
// WHY comment at the top of sync-lockfile-version.mjs for the full story.

import { describe, expect, it } from "vitest";
import { detectIndent, syncLockfileVersion } from "./sync-lockfile-version.mjs";

// A trimmed but structurally real package-lock.json, 4-space indented —
// matching this repo's actual convention (lockfileVersion 3 shape, a root
// package entry under packages[""], and a dependency entry with `resolved`/
// `integrity` that must survive untouched).
const FOUR_SPACE_LOCK = `{
    "name": "agentmux",
    "version": "0.56.10",
    "lockfileVersion": 3,
    "requires": true,
    "packages": {
        "": {
            "name": "agentmux",
            "version": "0.56.10",
            "license": "Apache-2.0",
            "dependencies": {
                "solid-js": "^1.9.0"
            }
        },
        "node_modules/solid-js": {
            "version": "1.9.0",
            "resolved": "https://registry.npmjs.org/solid-js/-/solid-js-1.9.0.tgz",
            "integrity": "sha512-fake=="
        }
    }
}
`;

const TWO_SPACE_LOCK = FOUR_SPACE_LOCK.replace(/ {4}/g, "  ").replace(/ {8}/g, "    ").replace(/ {12}/g, "      ");

describe("detectIndent", () => {
    it("detects 4-space indentation (this repo's convention)", () => {
        expect(detectIndent(FOUR_SPACE_LOCK)).toBe(4);
    });

    it("detects 2-space indentation (npm's default)", () => {
        expect(detectIndent(TWO_SPACE_LOCK)).toBe(2);
    });

    it("detects tab indentation", () => {
        const tabbed = '{\n\t"name": "x"\n}\n';
        expect(detectIndent(tabbed)).toBe("\t");
    });

    it("falls back to the repo's 4-space convention for un-sniffable input", () => {
        expect(detectIndent("{}")).toBe(4);
    });

    it("honours a caller-supplied fallback", () => {
        expect(detectIndent("{}", 2)).toBe(2);
    });
});

describe("syncLockfileVersion", () => {
    it("updates both version fields", () => {
        const { text } = syncLockfileVersion(FOUR_SPACE_LOCK, "0.56.11");
        const parsed = JSON.parse(text);
        expect(parsed.version).toBe("0.56.11");
        expect(parsed.packages[""].version).toBe("0.56.11");
    });

    it("leaves dependency entries byte-for-byte untouched", () => {
        const { text } = syncLockfileVersion(FOUR_SPACE_LOCK, "0.56.11");
        const parsed = JSON.parse(text);
        const dep = parsed.packages["node_modules/solid-js"];
        expect(dep.version).toBe("1.9.0");
        expect(dep.resolved).toBe("https://registry.npmjs.org/solid-js/-/solid-js-1.9.0.tgz");
        expect(dep.integrity).toBe("sha512-fake==");
    });

    // The regression test. Before the fix this failed: output was 2-space
    // regardless of input.
    it("preserves 4-space indentation on a 4-space input (the actual bug)", () => {
        const { text, indent } = syncLockfileVersion(FOUR_SPACE_LOCK, "0.56.11");
        expect(indent).toBe(4);
        expect(text).toMatch(/^\{\n {4}"name": "agentmux",\n {4}"version": "0\.56\.11",/);
        // Nested depth-2 lines must be 8 spaces (4 × 2), not npm's 2×2=4.
        expect(text).toMatch(/\n {8}"": \{\n {12}"name": "agentmux",/);
    });

    it("preserves 2-space indentation on a 2-space input (doesn't force 4 either)", () => {
        const { text, indent } = syncLockfileVersion(TWO_SPACE_LOCK, "0.56.11");
        expect(indent).toBe(2);
        expect(text).toMatch(/^\{\n {2}"name": "agentmux",\n {2}"version": "0\.56\.11",/);
    });

    it("does not touch packages[''] when the lockfile has no root package entry", () => {
        // Older/foreign lockfileVersion shapes may lack packages[""].
        // Must not throw — only the top-level version field is guaranteed.
        const noRootPkg = '{\n    "name": "x",\n    "version": "1.0.0",\n    "packages": {}\n}\n';
        const { text } = syncLockfileVersion(noRootPkg, "2.0.0");
        expect(JSON.parse(text).version).toBe("2.0.0");
    });

    it("produces output ending in exactly one trailing newline", () => {
        const { text } = syncLockfileVersion(FOUR_SPACE_LOCK, "0.56.11");
        expect(text.endsWith("\n")).toBe(true);
        expect(text.endsWith("\n\n")).toBe(false);
    });
});
