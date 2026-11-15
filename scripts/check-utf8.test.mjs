// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-utf8.mjs against a made-up git repo. The bad files are built from byte arrays here, so no file with an
// invalid byte is ever committed.

import { afterEach, describe, expect, it } from "vitest";
import { makeFixture } from "./lib/check-fixture.mjs";
import { utf8Problems } from "./check-utf8.mjs";

let fx;
afterEach(() => fx?.remove());

function repo(files) {
    fx = makeFixture(["check-utf8.mjs"], files).git();
    return fx.run("check-utf8.mjs");
}

const bytes = (...parts) => Buffer.concat(parts.map((p) => (typeof p === "string" ? Buffer.from(p, "utf8") : Buffer.from(p))));

describe("check-utf8", () => {
    it("passes valid UTF-8, including multi-byte text", () => {
        const r = repo({ "a.ts": "const s = \"§ € † 😀\";\n", "b.md": "plain\n" });
        expect(r.status).toBe(0);
        expect(r.out).toContain("check-utf8: ok");
    });

    it("reports a lone 0xA7 with its byte offset and line", () => {
        const r = repo({ "src/x.tsx": bytes("line one\nline two\n// see ", [0xa7], "3\n") });
        expect(r.status).toBe(1);
        // "line one\n" (9) + "line two\n" (9) + "// see " (7) = 25
        expect(r.out).toContain("src/x.tsx: byte offset 25, line 3: invalid UTF-8 byte 0xA7");
    });

    it("reports a byte-order mark", () => {
        const r = repo({ "c.json": bytes([0xef, 0xbb, 0xbf], "{}\n") });
        expect(r.status).toBe(1);
        expect(r.out).toContain("c.json: byte offset 0, line 1: byte-order mark");
    });

    it("skips a binary file (NUL byte) even with invalid bytes in it", () => {
        const r = repo({ "d.json": bytes([0x00, 0xa7, 0xff]) });
        expect(r.status).toBe(0);
    });

    it("skips extensions it does not check", () => {
        const r = repo({ "e.png": bytes([0xa7]), "f.txt": bytes([0xa7]) });
        expect(r.status).toBe(0);
    });

    it("skips an untracked file", () => {
        fx = makeFixture(["check-utf8.mjs"], { "ok.ts": "x\n" }).git();
        fx.write("new.ts", bytes([0xa7]));
        expect(fx.run("check-utf8.mjs").status).toBe(0);
    });
});

describe("utf8Problems", () => {
    it("rejects truncated, overlong and surrogate sequences, and accepts the limits", () => {
        expect(utf8Problems(Buffer.from([0xe2, 0x82]))[0].what).toContain("0xE2");
        expect(utf8Problems(Buffer.from([0xc0, 0x80]))[0].what).toContain("0xC0");
        expect(utf8Problems(Buffer.from([0xed, 0xa0, 0x80]))[0].what).toContain("0xED");
        expect(utf8Problems(Buffer.from([0xf4, 0x90, 0x80, 0x80]))[0].what).toContain("0xF4");
        expect(utf8Problems(Buffer.from([0xf4, 0x8f, 0xbf, 0xbf]))).toEqual([]);
        expect(utf8Problems(Buffer.from([0xc2, 0x80, 0xdf, 0xbf]))).toEqual([]);
    });

    it("counts lines across multi-byte characters", () => {
        const p = utf8Problems(bytes("€\n§\n", [0xa7]));
        expect(p).toEqual([{ offset: 7, line: 3, what: "invalid UTF-8 byte 0xA7" }]);
    });
});
