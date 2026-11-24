// @vitest-environment node
//
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// rotating_name.js: which hidden fields a page rewrites by itself (so their values stay out of the comparison
// that voids an approval), run as the same text act.rs sends.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "rotating_name.js"), "utf8");
const rotating = new Function(`return (${src});`)();

const token = "9f86d081884c7d659a2feaa0c55ad015";

describe("rotating_name: a hidden field the page rewrites by itself", () => {
    it("takes an anti-forgery token, a nonce, a timestamp and a honeypot, and says what kind of value stood there", () => {
        for (const n of ["csrf_token", "_csrf", "XSRF-TOKEN", "authenticity_token", "form_nonce", "_wpnonce", "g-recaptcha-response", "captcha_token"]) {
            expect(rotating(n, token), n).toBe("[rewritten by the page: hex 32]");
        }
        expect(rotating("timestamp", "1795518790438")).toBe("[rewritten by the page: digits 13]");
        expect(rotating("_ts", "1795518790")).toBe("[rewritten by the page: digits 10]");
        expect(rotating("csrf", "aB3+/x=_.-:Zz9Q")).toBe("[rewritten by the page: token 15]");
    });

    it("does not take a field that carries what is sent or to whom, whatever its value looks like", () => {
        for (const n of ["amount", "recipient", "account_number", "payment_token", "card_token", "price_total", "email", "user_id", "redirect", "plan", "next", "csrf_dest", "transfer_to", "token_url", "return_token"]) {
            expect(rotating(n, token), n).toBe("");
        }
    });

    it("does not take a value that is not shaped like a token, so a swapped value still voids the approval", () => {
        for (const v of ["https://evil.example/pay", "evil@example.com", "pay bob 500", "short", "", "a".repeat(513), "tok\nen12345"]) {
            expect(rotating("csrf_token", v), v).toBe("");
        }
    });

    it("gives a different stand-in when the value's kind or length changes", () => {
        const before = rotating("form_token", token);
        expect(rotating("form_token", "f".repeat(31))).not.toBe(before);
        expect(rotating("form_token", "x".repeat(32))).not.toBe(before);
        expect(rotating("form_token", "0123456789abcdef0123456789abcdee")).toBe(before);
    });

    it("takes nothing for an empty or missing name or a value that is not text", () => {
        expect(rotating("", token)).toBe("");
        expect(rotating(undefined, token)).toBe("");
        expect(rotating(null, token)).toBe("");
        expect(rotating("csrf", undefined)).toBe("");
    });
});
