// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the card form knows about a number as it is typed: its brand, whether
// it passes the Luhn check, how it is grouped, how long the security code is.
// srv's crates/srv/src/backend/vault/card.rs is the source (it works the brand
// out again on every save); vault-card.test.ts holds both to the same table.

export const CARD_BRANDS: Record<string, string> = {
    visa: "Visa",
    mastercard: "Mastercard",
    amex: "American Express",
    discover: "Discover",
    diners: "Diners Club",
    jcb: "JCB",
    unionpay: "UnionPay",
};

export function cardDigits(text: string): string {
    return text.replace(/\D/g, "");
}

/** The brand a number belongs to by its first digits, or an empty string. */
export function cardBrand(number: string): string {
    const d = cardDigits(number);
    const p = (n: number) => (d.length >= n ? Number(d.slice(0, n)) : -1);
    if (p(4) >= 3528 && p(4) <= 3589) return "jcb";
    if (p(2) === 34 || p(2) === 37) return "amex";
    if (p(4) === 6011 || (p(3) >= 644 && p(3) <= 649) || p(2) === 65) return "discover";
    if ((p(2) >= 51 && p(2) <= 55) || (p(4) >= 2221 && p(4) <= 2720)) return "mastercard";
    if ((p(3) >= 300 && p(3) <= 305) || p(2) === 36 || p(2) === 38) return "diners";
    if (p(2) === 62) return "unionpay";
    if (d.startsWith("4")) return "visa";
    return "";
}

/** Whether the digits pass the Luhn check. */
export function cardLuhn(number: string): boolean {
    const d = cardDigits(number);
    if (d.length < 2) return false;
    let sum = 0;
    for (let i = 0; i < d.length; i++) {
        let n = Number(d[d.length - 1 - i]);
        if (i % 2 === 1) {
            n *= 2;
            if (n > 9) n -= 9;
        }
        sum += n;
    }
    return sum % 10 === 0;
}

/** Grouped for reading: 4-6-5 for American Express, fours for the rest. */
export function cardGroup(number: string): string {
    const d = cardDigits(number);
    const sizes = cardBrand(d) === "amex" ? [4, 6, 5] : [4, 4, 4, 4, 4];
    const out: string[] = [];
    let rest = d;
    for (const size of sizes) {
        if (!rest) break;
        out.push(rest.slice(0, size));
        rest = rest.slice(size);
    }
    if (rest) out.push(rest);
    return out.join(" ");
}

/** Four digits for American Express, three for the rest. */
export function cardCvcLength(brand: string): number {
    return brand === "amex" ? 4 : 3;
}

/** The years the expiry select offers: this one and the twenty after it. */
export function expiryYears(now = new Date()): number[] {
    const y = now.getFullYear();
    return Array.from({ length: 21 }, (_, i) => y + i);
}

/** The month and year in a `YYYY-MM-DD` date, which is how srv gives a card's expiry (the last day of its month). */
export function expiryParts(expires: string | undefined): { month: string; year: string } | null {
    const m = /^(\d{4})-(\d{2})-\d{2}$/.exec(expires?.trim() ?? "");
    return m ? { month: m[2], year: m[1] } : null;
}
