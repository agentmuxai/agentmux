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

describe("rotating_name: a hidden field the page rewrites by itself", () => {
    it("takes an anti-forgery token, a nonce, a timestamp and a honeypot", () => {
        for (const n of ["csrf_token", "_csrf", "XSRF-TOKEN", "authenticity_token", "form_nonce", "_wpnonce", "timestamp", "ts", "_ts", "time", "hp", "honeypot", "g-recaptcha-response", "captcha_token"]) {
            expect(rotating(n), n).toBe(true);
        }
    });

    it("does not take a field that carries what is sent or to whom", () => {
        for (const n of ["amount", "recipient", "account_number", "payment_token", "card_token", "price_total", "email", "user_id", "redirect", "plan", "next"]) {
            expect(rotating(n), n).toBe(false);
        }
    });

    it("takes nothing for an empty or missing name", () => {
        expect(rotating("")).toBe(false);
        expect(rotating(undefined)).toBe(false);
        expect(rotating(null)).toBe(false);
    });
});
