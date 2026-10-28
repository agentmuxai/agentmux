// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What a service-OAuth account's card says beyond its stored status: the
// client it was connected with, a partial grant, a sign-in its organization
// blocked, and a Reconnect asked for from the card
// (docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md §7,
// "Status on the card" and "Some unticked").

import { createSignal } from "solid-js";

import type { Account } from "@/app/view/identity/identity-model";
import { supportsOAuth } from "./oauth-catalog";

/** An account connected through srv's service-OAuth flow (its tokens in the keychain). */
export function isServiceOAuthAccount(a: Pick<Account, "kind" | "provider" | "secret_ref">): boolean {
    return a.kind === "oauth" && a.secret_ref?.backend === "keychain" && supportsOAuth(a.provider);
}

/** The tag naming the user's own client; null for AgentMux's built-in one. */
export function clientModeTag(a: Pick<Account, "context">): string | null {
    const mode = a.context?.oauth_client?.mode;
    if (mode === "byo") return "Your own client";
    if (mode === "workspace") return "Workspace internal";
    return null;
}

/** Who a GitHub account is connected as ("@octo (id 42)"), or null when srv doesn't know. */
export function githubConnectedAs(a: Pick<Account, "provider" | "context">): string | null {
    const login = a.context?.github_login;
    if (a.provider !== "github" || !login) return null;
    const id = a.context?.github_user_id;
    return id ? `@${login} (id ${id})` : `@${login}`;
}

/** A GitHub grant made through AgentMux Cloud's broker, which the broker can say is gone. */
export function isBrokeredGithub(a: Pick<Account, "provider" | "context">): boolean {
    return a.provider === "github" && a.context?.broker === true;
}

/** Whether `github.connection`'s answer says the grant is gone (revoked on GitHub, blocked, or ended
 *  elsewhere): only a clear answer counts, never a failure to ask. */
export function githubGrantIsGone(c: { signedIn: boolean; error?: string; status?: string }): boolean {
    return c.signedIn && !c.error && (c.status === "revoked" || c.status === "blocked" || c.status === "none");
}

/** The label for a client mode `account.oauth.client.get` reports. */
export function clientModeLabel(mode: "byo" | "workspace" | undefined): string {
    return mode === "workspace" ? "Workspace internal" : "Your own client";
}

/** A flow error that says an organization's administrator blocks the app
 *  (srv maps Google's `admin_policy_enforced` to it). */
export function isBlockedError(error: string | null | undefined): boolean {
    return !!error && /blocked by your organization|admin_policy_enforced/i.test(error);
}

const GOOGLE_SCOPE_PREFIX = "https://www.googleapis.com/auth/";
const BASE_SCOPES = new Set([
    "openid",
    "email",
    "profile",
    `${GOOGLE_SCOPE_PREFIX}userinfo.email`,
    `${GOOGLE_SCOPE_PREFIX}userinfo.profile`,
]);

const SCOPE_LABELS: Record<string, string> = {
    openid: "Sign-in",
    email: "Email address",
    profile: "Basic profile",
    "userinfo.email": "Email address",
    "userinfo.profile": "Basic profile",
    "calendar.events": "Calendar events",
    "calendar.events.readonly": "Read calendar events",
    "calendar.calendarlist.readonly": "Calendar list",
    "calendar.events.freebusy": "Free/busy",
    "drive.file": "Drive files you pick",
    "drive.readonly": "Read all of Drive",
    "gmail.readonly": "Read Gmail",
    "gmail.compose": "Gmail drafts",
};

/** A permission's name in words: "Calendar events" for `…/auth/calendar.events`. */
export function scopeLabel(scope: string): string {
    const short = scope.startsWith(GOOGLE_SCOPE_PREFIX) ? scope.slice(GOOGLE_SCOPE_PREFIX.length) : scope;
    return SCOPE_LABELS[short] ?? short;
}

/** The line under a card when the consent screen left permissions unticked. */
export function partlyGrantedLine(missing: readonly string[]): string | null {
    if (missing.length === 0) return null;
    const names = [...new Set(missing.map(scopeLabel))];
    return `Partly granted: ${names.join(", ")} ${names.length === 1 ? "wasn't" : "weren't"} allowed`;
}

/** The scopes a Reconnect asks for again: what the account was granted
 *  beyond the base sign-in ones srv always asks for. */
export function reconnectScopes(a: Pick<Account, "context">): string[] {
    return (a.context?.granted_scopes ?? []).filter((s) => !BASE_SCOPES.has(s));
}

// ── Per-account notes from the last connect or reconnect ─────────────────────
//
// Kept for this session only: srv stores the granted scopes, not what was
// asked for, and a blocked sign-in changes nothing it stores.

export interface OAuthNote {
    /** Scopes asked for and not granted. */
    missingScopes?: string[];
    /** The flow's error when the organization blocks the app. */
    blocked?: string;
}

const [notes, setNotes] = createSignal<Record<string, OAuthNote>>({});

export function oauthNote(accountId: string): OAuthNote | undefined {
    return notes()[accountId];
}

export function setOAuthNote(accountId: string, note: OAuthNote | null): void {
    setNotes((prev) => {
        const next = { ...prev };
        if (note && (note.blocked || (note.missingScopes?.length ?? 0) > 0)) next[accountId] = note;
        else delete next[accountId];
        return next;
    });
}

/** A Reconnect is offered: the login stopped working, or its organization blocked the last try. */
export function canReconnect(a: Pick<Account, "id" | "kind" | "provider" | "secret_ref" | "status">): boolean {
    return isServiceOAuthAccount(a) && (a.status === "needs_reauth" || !!oauthNote(a.id)?.blocked);
}

// A Reconnect clicked on a card runs in the details that open under it.
const [reconnectRequest, setReconnectRequest] = createSignal<string | null>(null);

export function requestReconnect(accountId: string): void {
    setReconnectRequest(accountId);
}

/** True once for the account a card asked to reconnect, then cleared. */
export function takeReconnectRequest(accountId: string): boolean {
    if (reconnectRequest() !== accountId) return false;
    setReconnectRequest(null);
    return true;
}

export function pendingReconnectRequest(): string | null {
    return reconnectRequest();
}
