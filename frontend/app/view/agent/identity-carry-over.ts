// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Whether a carried-over identity/memory id from a prior agent instance
// looks like a real id worth reusing on a continuation/reattach launch.
//
// Real account ids and memory bundle ids are always UUID v4 strings
// (agentmux-srv/src/identity/oauth_client.rs, agentmux-srv/src/server/
// app_api/identity.rs). Legacy rows can carry "", the "blank" singleton,
// the pre-#1624-PR-C "default" sentinel, or any other now-meaningless
// literal from an older identity/memory scheme — none of these are ever
// real ids. A UUID-shape check classifies all of them (and any future,
// not-yet-observed legacy literal) as "no carry-over" in one place,
// instead of the incomplete per-literal blacklist (`!== "blank"` only)
// this replaces at both call sites. Forwarding an unresolvable id like
// "default" as an account_id causes a real backend failure —
// `linkagentidentity`'s FOREIGN KEY (account_id -> db_accounts(id))
// constraint — since no such row exists or should exist.
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export function looksLikeRealAccountId(id: string | null | undefined): boolean {
    return typeof id === "string" && UUID_RE.test(id);
}
