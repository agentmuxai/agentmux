// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { cardBrand, cardCvcLength, cardGroup, cardLuhn, expiryParts, expiryYears } from "./vault-card";

// The same numbers crates/srv/src/backend/vault/card.rs tests: the form's badge and srv's brand must agree.
const BRANDS: [string, string][] = [
    ["4111111111111111", "visa"],
    ["5555555555554444", "mastercard"],
    ["2221000000000009", "mastercard"],
    ["2720999999999996", "mastercard"],
    ["378282246310005", "amex"],
    ["341111111111111", "amex"],
    ["6011111111111117", "discover"],
    ["6445644564456445", "discover"],
    ["6500000000000002", "discover"],
    ["30569309025904", "diners"],
    ["36227206271667", "diners"],
    ["38520000023237", "diners"],
    ["3530111333300000", "jcb"],
    ["3589000000000000", "jcb"],
    ["6200000000000005", "unionpay"],
    ["9999999999999999", ""],
    ["", ""],
    ["2", ""],
];

describe("the card form's helpers", () => {
    it("name the brand by the first digits", () => {
        for (const [number, brand] of BRANDS) expect(cardBrand(number), number).toBe(brand);
        expect(cardBrand("4111 1111 1111 1111")).toBe("visa");
    });

    it("check the Luhn sum", () => {
        for (const ok of [
            "4111111111111111",
            "5555555555554444",
            "378282246310005",
            "6011111111111117",
            "30569309025904",
            "3530111333300000",
        ]) {
            expect(cardLuhn(ok), ok).toBe(true);
        }
        for (const bad of ["4111111111111112", "5555555555554445", "378282246310006", "1234567890123456", "7", ""]) {
            expect(cardLuhn(bad), bad).toBe(false);
        }
    });

    it("group a number as it is typed", () => {
        expect(cardGroup("4111111111111111")).toBe("4111 1111 1111 1111");
        expect(cardGroup("378282246310005")).toBe("3782 822463 10005");
        expect(cardGroup("41111")).toBe("4111 1");
        expect(cardGroup("4111-1111")).toBe("4111 1111");
        expect(cardGroup("")).toBe("");
    });

    it("give the security code four digits for American Express only", () => {
        expect(cardCvcLength("amex")).toBe(4);
        expect(cardCvcLength("visa")).toBe(3);
        expect(cardCvcLength("")).toBe(3);
    });

    it("offer this year and twenty more", () => {
        const years = expiryYears(new Date(2026, 10, 10));
        expect(years[0]).toBe(2026);
        expect(years).toHaveLength(21);
        expect(years[20]).toBe(2046);
    });

    it("read a card's expiry month and year from srv's date", () => {
        expect(expiryParts("2027-03-31")).toEqual({ month: "03", year: "2027" });
        expect(expiryParts("soon")).toBeNull();
        expect(expiryParts(undefined)).toBeNull();
    });
});
