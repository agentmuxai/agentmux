// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How a notice's trust is worded wherever it is shown: the Needs-you card, a jekt's tags and its origin row. A
 * signature that was genuine and failed only the age rules (`SIG=expired`: a notice held while its recipient or the
 * machine was away, delivered after its freshness window) is not a forged or unsigned message, so it says what
 * happened and is muted, not warned. This is wording and tone only: srv still treats such a notice as untrusted for
 * any action.
 */

import type { OriginSig } from "@/types/rpc/OriginSig";

export const SIG_EXPIRED_TEXT = "signature expired (arrived late)";

/** The card's state for a notice: a proven sender, an expired signature, or a sender nothing proved. */
export type TrustState = "proven" | "expired" | "unproven";

export function trustState(trust: string, sig: OriginSig | undefined): TrustState {
    if (sig === "expired") return "expired";
    return trust.endsWith("-verified") ? "proven" : "unproven";
}

/** The Needs-you card's line: how far the sender was proven. */
export function trustLine(trust: string, sig: OriginSig | undefined): string {
    switch (trustState(trust, sig)) {
        case "expired":
            return `Signature expired (arrived late), trust: ${trust}`;
        case "proven":
            return `Sender proven (${trust})`;
        default:
            return `Sender not proven (${trust})`;
    }
}

/** The short signature label of a message's origin row. */
export function sigLabel(sig: OriginSig): string {
    return sig === "verified" ? "signed" : sig === "expired" ? SIG_EXPIRED_TEXT : "signature invalid";
}

/** Colour marks a trust problem only: an expired signature is a late notice, not one that did not hold. */
export function sigTone(sig: OriginSig): "neutral" | "warn" {
    return sig === "verified" || sig === "expired" ? "neutral" : "warn";
}
