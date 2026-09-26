// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { redactPrivateKeys, redactSecrets } from "./redact";

const here = dirname(fileURLToPath(import.meta.url));

interface RedactionVector {
    name: string;
    input: string;
    expected: string;
}

function loadVectors(): RedactionVector[] {
    const path = join(here, "..", "..", "..", "docs", "specs", "fixtures", "redaction-vectors.json");
    return JSON.parse(readFileSync(path, "utf-8"));
}

describe("redactSecrets — shared vectors (docs/specs/fixtures/redaction-vectors.json)", () => {
    const vectors = loadVectors();
    it("loaded at least one vector", () => {
        expect(vectors.length).toBeGreaterThan(0);
    });
    for (const v of vectors) {
        it(`${v.name}`, () => {
            expect(redactSecrets(v.input)).toBe(v.expected);
        });
    }
});

describe("redactSecrets — additional cases not in the shared fixture", () => {
    it("a private key after a certificate is still redacted", () => {
        const text =
            "-----BEGIN CERTIFICATE-----\nMIIBcert\n-----END CERTIFICATE-----\n" +
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEsecret\n-----END RSA PRIVATE KEY-----\ndone";
        const out = redactPrivateKeys(text);
        expect(out).toContain("MIIBcert");
        expect(out).not.toContain("MIIEsecret");
        expect(out).toContain("[redacted private key]");
        expect(out.endsWith("done")).toBe(true);
    });

    it("a secret straddling a truncation cap is still redacted", () => {
        const long = "a".repeat(1_994) + "ghp_abcdefghijklmnopqrstuvwxyz0123" + "b".repeat(5_000);
        const out = redactSecrets(long);
        expect(out).not.toContain("ghp_ab");
        expect(out).toContain("[redacted secret]");
    });

    it("every Slack token family is redacted", () => {
        for (const prefix of ["xoxa-", "xoxb-", "xoxo-", "xoxp-", "xoxr-", "xoxs-"]) {
            const token = `${prefix}1234567890-abcdefghijklmnop`;
            expect(redactSecrets(`t ${token} t`)).toBe("t [redacted secret] t");
        }
    });

    it("leaves a near-miss (short body) prefix untouched", () => {
        expect(redactSecrets("sk-short")).toBe("sk-short");
    });
});
