// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The API block's `auth`: a Vault entry whose value is sent as one header, `{ vault, as }`
 * with `as` "bearer" or "header:<Name>" (crates/srv drone/executor/blocks/api_auth.rs). The
 * inspector edits it as three fields; the node keeps the one object. The value is never here:
 * the first call asks the person, in AgentMux.
 */

export type ApiAuthHow = "bearer" | "header";

export interface ApiAuthFields {
    /** The Vault entry, `id` or `id.part`; empty for none. */
    vault: string;
    how: ApiAuthHow;
    /** The header's name, when `how` is "header". */
    header: string;
}

/** The fields a node's `auth` shows as. */
export function apiAuthFields(data: Record<string, unknown>): ApiAuthFields {
    const auth = data["auth"];
    const obj = auth && typeof auth === "object" ? (auth as Record<string, unknown>) : {};
    const vault = typeof obj["vault"] === "string" ? (obj["vault"] as string) : "";
    const as = typeof obj["as"] === "string" ? (obj["as"] as string) : "bearer";
    if (as.startsWith("header:")) return { vault, how: "header", header: as.slice("header:".length) };
    return { vault, how: "bearer", header: "" };
}

/** The node-data patch that writes `fields`: no `auth` at all when there is no entry. */
export function apiAuthPatch(fields: ApiAuthFields): Record<string, unknown> {
    const vault = fields.vault.trim();
    if (!vault) return { auth: undefined };
    return { auth: { vault, as: fields.how === "header" ? `header:${fields.header.trim()}` : "bearer" } };
}
