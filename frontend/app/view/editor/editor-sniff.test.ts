// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What the editor will not hand to CodeMirror, and which of it the Large text viewer shows. */

import { describe, expect, it } from "vitest";
import { sniffRefusal, sniffUnopenable } from "./editor-sniff";

describe("sniffRefusal", () => {
    it("says binary for a NUL in the first bytes, wide for a first line over 100,000 characters, and null for text", () => {
        expect(sniffRefusal(`ab${String.fromCharCode(0)}cd`)?.kind).toBe("binary");
        expect(sniffRefusal("x".repeat(100_001))?.kind).toBe("wide");
        expect(sniffRefusal(`short\n${"x".repeat(200_000)}`)).toBeNull();
        expect(sniffRefusal("x".repeat(100_000))).toBeNull();
    });

    it("calls a wide line of binary binary, and lets the person's choice of text past NUL but never past a wide line", () => {
        expect(sniffRefusal(`${String.fromCharCode(0)}${"x".repeat(100_001)}`)?.kind).toBe("binary");
        expect(sniffRefusal(`${String.fromCharCode(0)}ab`, true)).toBeNull();
        expect(sniffRefusal("x".repeat(100_001), true)?.kind).toBe("wide");
    });

    it("keeps sniffUnopenable's message", () => {
        expect(sniffUnopenable("x".repeat(100_001))).toMatch(/first line is 100,001 characters/);
        expect(sniffUnopenable("fine\n")).toBeNull();
    });
});
