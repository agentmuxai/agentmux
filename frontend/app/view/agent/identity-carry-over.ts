// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Whether a carried-over identity id from a prior agent instance looks
// like a real account id worth reusing on a continuation/reattach launch.
//
// ONLY account ids — this is NOT a general-purpose "is this a real id"
// check, and is not valid for bundle ids (`db_bundles` seeds a
// permanent `id='blank'` row and reserves a `seed-*` id prefix for
// workspace-default bundles — both legitimate, both non-UUID; see
// storage/bundles.rs / app_api/bundle/mod.rs), so never apply this check to
// a memory_id (#2464).
//
// Real account ids ARE always UUID v4 strings (crates/srv/src/identity/
// oauth_client.rs, crates/srv/src/server/app_api/identity.rs). Legacy
// rows can carry "", the "blank" singleton, the pre-#1624-PR-C "default"
// sentinel, or any other now-meaningless literal from an older identity
// scheme — none of these are ever real account ids. A UUID-shape check
// classifies all of them (and any future, not-yet-observed legacy
// literal) as "no carry-over" in one place, instead of the incomplete
// per-literal blacklist (`!== "blank"` only) this replaces at both call
// sites. Forwarding an unresolvable id like "default" as an account_id
// causes a real backend failure — `linkagentidentity`'s FOREIGN KEY
// (account_id -> db_accounts(id)) constraint — since no such row exists
// or should exist.
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export function looksLikeRealAccountId(id: string | null | undefined): boolean {
    return typeof id === "string" && UUID_RE.test(id);
}

// A UUID shape alone isn't sufficient to trust a carried-over account id:
// pre-#1624-PR-C `db_identity_bundles` ids were ALSO UUID-formatted, just not
// account ids (codex P1 on #2464) — a legacy row carrying one would pass
// `looksLikeRealAccountId` but still fail to resolve as a real account,
// hitting the same FOREIGN KEY failure this module exists to prevent.
// Cross-checking against the caller's own known-accounts list closes that
// gap. Takes the id list as a parameter (rather than reading a cache
// internally) so this stays a pure, directly-testable function — callers
// pass `loadAccounts().map(a => a.id)` (identity-model.ts) or equivalent.
export function realAccountIdOrEmpty(id: string, knownAccountIds: readonly string[]): string {
    if (!looksLikeRealAccountId(id)) return "";
    return knownAccountIds.includes(id) ? id : "";
}
