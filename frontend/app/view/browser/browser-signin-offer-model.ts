// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The "Save to Vault?" offer as srv writes it to a browser pane's
// `browser:signin_offer` meta, and the question it asks (browser-signin-offer.tsx);
// the saved sign-ins srv lists in `browser:signins` for the key button
// (browser-signin-key.tsx).

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

/** A saved sign-in the page's form can take: never its password. */
export interface SavedSignin {
    entry: string;
    origin: string;
    /** Masked, e.g. `al***`; empty when it was saved without a user name. */
    user: string;
}

export function readSavedSignins(meta: Record<string, unknown> | undefined): SavedSignin[] {
    const v = meta?.["browser:signins"];
    if (!Array.isArray(v)) return [];
    return v.flatMap((s: Partial<SavedSignin> | null) =>
        s && typeof s.entry === "string" && typeof s.origin === "string"
            ? [{ entry: s.entry, origin: s.origin, user: typeof s.user === "string" ? s.user : "" }]
            : []
    );
}

/** "Sign in as al***", naming the site too when the page has forms of more than one. */
export function savedSigninLabel(s: SavedSignin, all: SavedSignin[]): string {
    const who = s.user ? `Sign in as ${s.user}` : "Sign in (no user name)";
    return all.some((o) => o.origin !== s.origin) ? `${who} on ${s.origin}` : who;
}
