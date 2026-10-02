// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { isUsableTitle } from "./ambient-title";

// The one corpus the Rust validator also loads (crates/srv/src/ambient/validate.rs,
// `the_shared_corpus_is_honoured`), so the two implementations cannot drift.
const corpus = JSON.parse(
    readFileSync(join(__dirname, "../../../crates/srv/src/ambient/title_corpus.json"), "utf8"),
) as { rejected: string[]; accepted: string[] };

describe("isUsableTitle", () => {
    it("loads a corpus that has not been hollowed out", () => {
        expect(corpus.rejected.length).toBeGreaterThanOrEqual(30);
        expect(corpus.accepted.length).toBeGreaterThanOrEqual(10);
    });

    it("rejects everything the shared corpus lists as about the absence of a title", () => {
        for (const s of corpus.rejected) {
            expect(isUsableTitle(s), `must reject ${JSON.stringify(s)}`).toBe(false);
        }
    });

    it("accepts every real title in the corpus, including ones containing an absence-like word", () => {
        for (const s of corpus.accepted) {
            expect(isUsableTitle(s), `must accept ${JSON.stringify(s)}`).toBe(true);
        }
    });

    // The two values actually found in the owner's database on 2026-10-02.
    it("rejects the values found in the wild, and accepts the real title that replaced them", () => {
        expect(isUsableTitle("(none yet)")).toBe(false);
        expect(isUsableTitle("no goal established yet")).toBe(false);
        expect(isUsableTitle("Develop hardening spec for swarm ambient summary quality")).toBe(true);
    });

    it("never treats the abstain token as a title", () => {
        for (const s of ["KEEP", "keep", "Keep.", "(KEEP)", " KEEP "]) {
            expect(isUsableTitle(s), s).toBe(false);
        }
    });

    it("rejects empty, missing, multi-line and over-long text", () => {
        for (const s of [undefined, null, "", "   ", "...", "-", "Fix login\nthen push"]) {
            expect(isUsableTitle(s as string | null | undefined), String(s)).toBe(false);
        }
        expect(isUsableTitle(Array(40).fill("word").join(" "))).toBe(false);
        expect(isUsableTitle("x".repeat(201))).toBe(false);
    });

    it("matches whole words, so a real title that starts with an absence word survives", () => {
        for (const s of ["None of the tests pass", "Untitled-tab bug", "Keep alive pings"]) {
            expect(isUsableTitle(s), s).toBe(true);
        }
    });
});
