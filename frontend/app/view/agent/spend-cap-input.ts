// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What a person types as an agent's daily spend cap (AgentSpendPanel.tsx): dollars, with or without a "$". The least a cap
// may be is what srv takes (crates/srv-core/src/backend/model_gateway/cap.rs, MIN_CAP_USD); "no cap" is its own button.

export const MIN_CAP_USD = 0.01;

export type CapInput = { ok: true; usd: number } | { ok: false; reason: "empty" | "number" | "least" };

/** The dollars in the text, or why it is not a cap. */
export function parseCapInput(text: string): CapInput {
    const trimmed = text.trim().replace(/^\$/, "").trim();
    if (trimmed === "") return { ok: false, reason: "empty" };
    if (!/^\d+(\.\d+)?$/.test(trimmed)) return { ok: false, reason: "number" };
    const usd = Number(trimmed);
    if (!Number.isFinite(usd)) return { ok: false, reason: "number" };
    if (usd < MIN_CAP_USD) return { ok: false, reason: "least" };
    return { ok: true, usd };
}

/** A figure of dollars rounded to the cent, as a number: the dictionary writes a number in the person's own format. */
export function cents(usd: number): number {
    return Math.round(usd * 100) / 100;
}
