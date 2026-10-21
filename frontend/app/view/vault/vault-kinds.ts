// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the Vault can hold, for the pane's forms
// (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.2).
// srv's catalog (crates/srv/src/backend/vault/kinds.rs) is the source: it
// refuses a part or field a kind doesn't have, and vault-kinds.test.ts checks
// this list names the same kinds, parts and fields.

import type { PaneSections } from "@/app/block/pane-sections";
import type { VaultSection } from "@/types/rpc/VaultSection";

export type VaultPaneSection = VaultSection | "kept" | "activity";

export interface VaultInput {
    name: string;
    label: string;
    /** Shown in the empty input. */
    placeholder?: string;
    /** A longer text: a textarea. */
    multiline?: boolean;
}

export interface VaultKind {
    name: string;
    label: string;
    section: VaultSection;
    /** Secret: typed hidden, sealed by srv, never shown back. */
    parts: VaultInput[];
    /** Plain: shown in the pane. */
    fields: VaultInput[];
}

const OWNER: VaultInput = { name: "owner", label: "Whose", placeholder: "e.g. Alex Doe" };
const NOTES: VaultInput = { name: "notes", label: "Notes", multiline: true };
const ID_FIELDS: VaultInput[] = [
    OWNER,
    { name: "issuer", label: "Issued by", placeholder: "Country or state" },
    { name: "expires", label: "Expires", placeholder: "YYYY-MM-DD" },
    NOTES,
];
const value = (label: string, placeholder?: string): VaultInput[] => [{ name: "value", label, placeholder }];

export const VAULT_KINDS: VaultKind[] = [
    { name: "ssn", label: "Social Security number", section: "personal", parts: value("Number", "123-45-6789"), fields: [OWNER, NOTES] },
    { name: "itin", label: "ITIN", section: "personal", parts: value("Number"), fields: [OWNER, NOTES] },
    { name: "tin", label: "Taxpayer ID", section: "personal", parts: value("Number"), fields: [OWNER, NOTES] },
    { name: "ein", label: "EIN", section: "personal", parts: value("Number", "12-3456789"), fields: [OWNER, NOTES] },
    { name: "date-of-birth", label: "Date of birth", section: "personal", parts: value("Date", "YYYY-MM-DD"), fields: [OWNER, NOTES] },
    { name: "passport", label: "Passport", section: "personal", parts: value("Passport number"), fields: ID_FIELDS },
    { name: "drivers-licence", label: "Driver's licence", section: "personal", parts: value("Licence number"), fields: ID_FIELDS },
    { name: "other-id", label: "Other ID number", section: "personal", parts: value("Number"), fields: ID_FIELDS },
    {
        name: "card",
        label: "Payment card",
        section: "cards",
        parts: [
            { name: "number", label: "Card number" },
            { name: "expiry", label: "Expiry", placeholder: "MM/YY" },
            { name: "cvc", label: "Security code" },
            { name: "name", label: "Name on card" },
        ],
        fields: [OWNER, NOTES],
    },
    {
        name: "bank-account",
        label: "Bank account",
        section: "cards",
        parts: [
            { name: "account", label: "Account number" },
            { name: "routing", label: "Routing number" },
        ],
        fields: [OWNER, { name: "bank", label: "Bank" }, NOTES],
    },
    {
        name: "api-key",
        label: "API key or token",
        section: "api-keys",
        parts: value("Key"),
        fields: [
            { name: "service", label: "Service", placeholder: "e.g. Stripe" },
            { name: "env_name", label: "Environment variable", placeholder: "e.g. STRIPE_API_KEY" },
            { name: "expires", label: "Expires", placeholder: "YYYY-MM-DD" },
            NOTES,
        ],
    },
    {
        name: "login",
        label: "Login",
        section: "logins",
        parts: [
            { name: "password", label: "Password" },
            { name: "totp", label: "One-time code seed", placeholder: "otpauth:// or base32" },
        ],
        fields: [{ name: "site", label: "Site", placeholder: "https://example.com" }, { name: "username", label: "Username" }, NOTES],
    },
    { name: "note", label: "Secret note", section: "notes", parts: [{ name: "text", label: "Text", multiline: true }], fields: [] },
];

export const SECTION_LABELS: Record<VaultPaneSection, string> = {
    personal: "Personal data",
    cards: "Cards and banks",
    "api-keys": "API keys",
    logins: "Logins",
    notes: "Notes",
    kept: "Kept by AgentMux",
    activity: "Activity",
};

export function isVaultPaneSection(v: unknown): v is VaultPaneSection {
    return typeof v === "string" && Object.prototype.hasOwnProperty.call(SECTION_LABELS, v);
}

/** The Vault's sections, in rail order, as its manifest declares them (pane-sections.ts). */
export const VAULT_SECTIONS: PaneSections<VaultPaneSection> = {
    default: "personal",
    items: [
        { id: "personal", label: SECTION_LABELS.personal, icon: "id-card" },
        { id: "cards", label: SECTION_LABELS.cards, icon: "credit-card" },
        { id: "api-keys", label: SECTION_LABELS["api-keys"], icon: "key" },
        { id: "logins", label: SECTION_LABELS.logins, icon: "right-to-bracket" },
        { id: "notes", label: SECTION_LABELS.notes, icon: "note-sticky" },
        { id: "kept", label: SECTION_LABELS.kept, icon: "box-archive" },
        { id: "activity", label: SECTION_LABELS.activity, icon: "list-check" },
    ],
};

export function vaultKind(name: string): VaultKind | undefined {
    return VAULT_KINDS.find((k) => k.name === name);
}

export function kindsIn(section: VaultSection): VaultKind[] {
    return VAULT_KINDS.filter((k) => k.section === section);
}
