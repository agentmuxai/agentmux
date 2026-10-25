// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The SDK's `agentmux-widget` signer (sdk/widget-sdk/cli/widget-sign.mjs, run
 * by the `agentmux-widget.mjs` command)
 * against the fixture srv verifies in `widget_signature.rs`
 * (`the_sdk_signed_fixture_verifies_here`): both must compute the same hash,
 * signature and fingerprint (docs/specs/SPEC_WIDGET_SHARING_2026_10_10.md §2.4).
 */

import { execFileSync } from "node:child_process";
import { createPublicKey, verify } from "node:crypto";
import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { fingerprint, keyFromSeed, packageHash, signedMessage, signPackage, verifyPackage } from "../../../sdk/widget-sdk/cli/widget-sign.mjs";

const FIXTURE = join(__dirname, "../../../sdk/widget-sdk/fixtures/acme.fixture");
const TEMPLATE_FIXTURE = join(__dirname, "../../../sdk/widget-sdk/fixtures/acme.template-fixture");
const BIN = join(__dirname, "../../../sdk/widget-sdk/cli/agentmux-widget.mjs");
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

    it("runs as a command, keygen, sign and verify, however it's started", () => {
        const dir = join(mkdtempSync(join(tmpdir(), "widget-sign-")), "acme.fixture");
        cpSync(FIXTURE, dir, { recursive: true });
        const key = join(dir, "..", "key.json");
        const run = (...args: string[]) => execFileSync(process.execPath, [BIN, ...args], { encoding: "utf8" });
        expect(run("keygen", key)).toMatch(/fingerprint: [A-Z2-7]{4}-/);
        expect(run("sign", dir, "--key", key)).toMatch(/^Signed acme\.fixture 1\.0\.0/);
        const fp = fingerprint(JSON.parse(readFileSync(key, "utf8")).publicKey);
        expect(run("verify", dir).trim()).toBe(`acme.fixture: signed by ${fp}`);
    });

    // Job templates (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.4), checked
    // in srv by `the_sdk_signed_template_fixture_verifies_here`.
    it("signs a folder with a template.json into template.sig, under the template prefix", () => {
        expect(packageHash(TEMPLATE_FIXTURE)).toBe("9c78ceeea2cf6db5e70cde3521bc24c971a7ef39fd73afb9e1050b7a7c89b761");
        expect(signPackage(TEMPLATE_FIXTURE, SEED)).toBe(readFileSync(join(TEMPLATE_FIXTURE, "template.sig"), "utf8"));
        expect(verifyPackage(TEMPLATE_FIXTURE)).toBe("72AS-YEXT-VNGO-NLC5");
    });

    it("keeps a template's signature from passing as a widget's", () => {
        const sig = JSON.parse(readFileSync(join(TEMPLATE_FIXTURE, "template.sig"), "utf8"));
        const key = createPublicKey(keyFromSeed(SEED).privateKey);
        const signature = Buffer.from(sig.signature, "base64");
        const hash = packageHash(TEMPLATE_FIXTURE);
        const asTemplate = signedMessage("acme.template-fixture", "1.0.0", hash, "agentmux-job-template-sig-v1\0");
        expect(verify(null, asTemplate, key, signature)).toBe(true);
        expect(verify(null, signedMessage("acme.template-fixture", "1.0.0", hash), key, signature)).toBe(false);
    });

    it("signs a template from the command line", () => {
        const dir = join(mkdtempSync(join(tmpdir(), "template-sign-")), "acme.template-fixture");
        cpSync(TEMPLATE_FIXTURE, dir, { recursive: true });
        rmSync(join(dir, "template.sig"));
        const key = join(dir, "..", "key.json");
        writeFileSync(key, JSON.stringify({ algorithm: "ed25519", seed: SEED, publicKey: keyFromSeed(SEED).publicKey }));
        const out = execFileSync(process.execPath, [BIN, "sign", dir, "--key", key], { encoding: "utf8" });
        expect(out).toMatch(/^Signed acme\.template-fixture 1\.0\.0/);
        expect(readFileSync(join(dir, "template.sig"), "utf8")).toBe(readFileSync(join(TEMPLATE_FIXTURE, "template.sig"), "utf8"));
        expect(existsSync(join(dir, "widget.sig"))).toBe(false);
    });
});
