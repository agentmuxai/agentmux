// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The "Save to Vault?" offer as srv writes it to a browser pane's
// `browser:signin_offer` meta, and the question it asks (browser-signin-offer.tsx).

export interface SigninOffer {
    id: string;
    origin: string;
    /** Masked, e.g. `al***`; empty when the page sent no user name. */
    user: string;
    /** The profile holds another password for this user. */
    update: boolean;
}

export function readSigninOffer(meta: Record<string, unknown> | undefined): SigninOffer | undefined {
    const v = meta?.["browser:signin_offer"] as Partial<SigninOffer> | null | undefined;
    return v && typeof v === "object" && typeof v.id === "string" && typeof v.origin === "string"
        ? { id: v.id, origin: v.origin, user: typeof v.user === "string" ? v.user : "", update: v.update === true }
        : undefined;
}

export function signinOfferQuestion(o: SigninOffer): string {
    const who = o.user ? ` (${o.user})` : "";
    return o.update ? `Update the saved password for ${o.origin}${who}?` : `Save the sign-in for ${o.origin}${who} to the Vault?`;
}
