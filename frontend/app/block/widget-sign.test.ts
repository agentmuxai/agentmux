// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The SDK's `agentmux-widget` signer (sdk/widget-sdk/cli/agentmux-widget.mjs)
 * against the fixture srv verifies in `widget_signature.rs`
 * (`the_sdk_signed_fixture_verifies_here`): both must compute the same hash,
 * signature and fingerprint (docs/specs/SPEC_WIDGET_SHARING_2026_10_10.md §2.4).
 */

import { cpSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { fingerprint, keyFromSeed, packageHash, signPackage, verifyPackage } from "../../../sdk/widget-sdk/cli/agentmux-widget.mjs";

const FIXTURE = join(__dirname, "../../../sdk/widget-sdk/fixtures/acme.fixture");
const SEED = Buffer.alloc(32, 7).toString("base64");

describe("agentmux-widget", () => {
    it("hashes the fixture as AgentMux does, leaving widget.sig out", () => {
        expect(packageHash(FIXTURE)).toBe("2f480223aad83d3e220179f1e67c8bd660d5bec197cd334b71f514661018b202");
    });

    it("signs it to the very widget.sig srv verifies, with the same fingerprint", () => {
        expect(signPackage(FIXTURE, SEED)).toBe(readFileSync(join(FIXTURE, "widget.sig"), "utf8"));
        expect(verifyPackage(FIXTURE)).toBe("72AS-YEXT-VNGO-NLC5");
        expect(fingerprint(keyFromSeed(SEED).publicKey)).toBe("72AS-YEXT-VNGO-NLC5");
    });

    it("refuses a package edited after it was signed", () => {
        const dir = join(mkdtempSync(join(tmpdir(), "widget-sign-")), "acme.fixture");
        cpSync(FIXTURE, dir, { recursive: true });
        writeFileSync(join(dir, "index.html"), "<p>edited</p>\n");
        expect(() => verifyPackage(dir)).toThrow(/doesn't match its files/);
    });
});
