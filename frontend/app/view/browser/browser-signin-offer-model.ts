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

/** A field for a new password the person is in (§6.2), as srv writes `browser:signup`. */
export interface SignupOffer {
    origin: string;
}

export function readSignupOffer(meta: Record<string, unknown> | undefined): SignupOffer | undefined {
    const v = meta?.["browser:signup"] as Partial<SignupOffer> | null | undefined;
    return v && typeof v === "object" && typeof v.origin === "string" ? { origin: v.origin } : undefined;
}

export const STRONG_PASSWORD_LABEL = "Use a strong password (saved to the Vault)";

/** "<agent> is signing in", shown before a sign-in under the person's "Always" is typed. */
export interface SigninPending {
    id: string;
    agent: string;
    origin: string;
    /** Masked, e.g. `al***`. */
    user: string;
    /** `account`: an agent making a new account; `change`: an agent changing a saved password; otherwise signing in. */
    kind: "signin" | "account" | "change";
    deadlineMs: number;
}

export function readSigninPending(meta: Record<string, unknown> | undefined): SigninPending | undefined {
    const v = meta?.["browser:signin_pending"] as Record<string, unknown> | null | undefined;
    if (!v || typeof v !== "object") return undefined;
    const { id, agent, origin, user, deadline_ms } = v;
    if (typeof id !== "string" || typeof agent !== "string" || typeof origin !== "string" || typeof deadline_ms !== "number") return undefined;
    return { id, agent, origin, user: typeof user === "string" ? user : "", kind: v.kind === "account" || v.kind === "change" ? v.kind : "signin", deadlineMs: deadline_ms };
}

export function signinPendingText(p: SigninPending, nowMs: number): string {
    const left = Math.max(0, Math.ceil((p.deadlineMs - nowMs) / 1000));
    if (p.kind === "account") return `${p.agent} is creating an account on ${p.origin}: the password comes from your Vault (${left} s)`;
    if (p.kind === "change") return `${p.agent} is changing your password on ${p.origin}: the new one comes from your Vault and the old one is kept for 7 days (${left} s)`;
    const as = p.user ? ` as ${p.user}` : "";
    return `${p.agent} is signing in to ${p.origin}${as} with your saved sign-in (${left} s)`;
}

/** The accessible name of the list of saved sign-ins under a login field. */
export const SIGNIN_LIST_LABEL = "Saved sign-ins for this page";
export const MANAGE_PASSWORDS_LABEL = "Manage passwords";

/** The list of saved sign-ins under a login field the person is in, as srv writes `browser:signin-list`
 *  (docs/specs/SPEC_VAULT_AUTOFILL_IN_BROWSER_PANE_2026_11_14.md §P2.3): masked user names, never a value. */
export interface SigninList {
    origin: string;
    role: "username" | "password" | "newpassword";
    rows: { entry: string; user: string }[];
    /** A strong password is offered: the only item, for a field for a new password. */
    strong: boolean;
    /** The selected item, by the keys; none until Up or Down. */
    cursor: number | null;
}

export function readSigninList(meta: Record<string, unknown> | undefined): SigninList | undefined {
    const v = meta?.["browser:signin-list"] as Record<string, unknown> | null | undefined;
    if (!v || typeof v !== "object" || typeof v.origin !== "string" || !Array.isArray(v.rows)) return undefined;
    const role = v.role === "password" || v.role === "newpassword" ? v.role : "username";
    const rows = v.rows.flatMap((r: { entry?: unknown; user?: unknown } | null) =>
        r && typeof r.entry === "string" ? [{ entry: r.entry, user: typeof r.user === "string" ? r.user : "" }] : []
    );
    const strong = v.strong === true;
    if (!strong && rows.length === 0) return undefined;
    return { origin: v.origin, role, rows, strong, cursor: typeof v.cursor === "number" ? v.cursor : null };
}

/** What a row says: the masked user name, or that the sign-in has none. */
export function listRowLabel(row: { user: string }): string {
    return row.user || "No user name";
}
