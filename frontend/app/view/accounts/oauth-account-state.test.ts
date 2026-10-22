// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What a service-OAuth account's card says beyond its status
// (docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md §7).

import { afterEach, describe, expect, it } from "vitest";

import type { Account } from "@/app/view/identity/identity-model";
import {
    canReconnect,
    clientModeTag,
    isBlockedError,
    isServiceOAuthAccount,
    oauthNote,
    partlyGrantedLine,
    reconnectScopes,
    requestReconnect,
    scopeLabel,
    setOAuthNote,
    takeReconnectRequest,
} from "./oauth-account-state";

const CAL = "https://www.googleapis.com/auth/calendar.events";

function google(over: Partial<Account> = {}): Account {
    return {
        id: "g1",
        name: "google",
        provider: "google",
        kind: "oauth",
        secret_ref: { backend: "keychain", service: "agentmux", account: "g1" },
        context: { email: "me@example.com", oauth_client: { mode: "byo", client_id: "cid" } },
        assigned_agents: [],
        status: "valid",
        created_at: "0",
        updated_at: "0",
        ...over,
    };
}

afterEach(() => {
    setOAuthNote("g1", null);
    takeReconnectRequest("g1");
});

describe("service-OAuth accounts", () => {
    it("are OAuth accounts whose tokens srv keeps in the keychain", () => {
        expect(isServiceOAuthAccount(google())).toBe(true);
        // A CLI login (Claude's config dir) isn't one, nor is a key.
        expect(isServiceOAuthAccount(google({ provider: "claude" as Account["provider"], secret_ref: { backend: "oauth_config_dir" } }))).toBe(false);
        expect(isServiceOAuthAccount(google({ kind: "api_key" }))).toBe(false);
    });

    it("tag the user's own client and a Workspace internal one, not AgentMux's", () => {
        expect(clientModeTag(google())).toBe("Your own client");
        expect(clientModeTag(google({ context: { oauth_client: { mode: "workspace" } } }))).toBe("Workspace internal");
        expect(clientModeTag(google({ context: { oauth_client: { mode: "builtin" } } }))).toBeNull();
        expect(clientModeTag(google({ context: {} }))).toBeNull();
    });

    it("recognize the organization's block srv reports for admin_policy_enforced", () => {
        expect(
            isBlockedError(
                "blocked by your organization: a google Workspace administrator doesn't allow this app (admin_policy_enforced).",
            ),
        ).toBe(true);
        expect(isBlockedError("access denied: the consent screen was cancelled or declined")).toBe(false);
        expect(isBlockedError(undefined)).toBe(false);
    });

    it("name permissions in words and say which weren't granted", () => {
        expect(scopeLabel(CAL)).toBe("Calendar events");
        expect(scopeLabel("email")).toBe("Email address");
        expect(scopeLabel("https://www.googleapis.com/auth/unknown.scope")).toBe("unknown.scope");
        expect(partlyGrantedLine([])).toBeNull();
        expect(partlyGrantedLine([CAL])).toBe("Partly granted: Calendar events wasn't allowed");
        expect(partlyGrantedLine([CAL, "email", "https://www.googleapis.com/auth/userinfo.email"])).toBe(
            "Partly granted: Calendar events, Email address weren't allowed",
        );
    });

    it("reconnect for the scopes the account had beyond the base sign-in", () => {
        const a = google({
            context: { granted_scopes: ["openid", "https://www.googleapis.com/auth/userinfo.email", CAL] },
        });
        expect(reconnectScopes(a)).toEqual([CAL]);
        expect(reconnectScopes(google({ context: {} }))).toEqual([]);
    });

    it("offer Reconnect when the login stopped working or the organization blocked it", () => {
        expect(canReconnect(google())).toBe(false);
        expect(canReconnect(google({ status: "needs_reauth" }))).toBe(true);
        setOAuthNote("g1", { blocked: "blocked by your organization" });
        expect(canReconnect(google())).toBe(true);
        // A CLI login can't be reconnected from here.
        expect(canReconnect(google({ id: "c1", secret_ref: { backend: "oauth_config_dir" }, status: "needs_reauth" }))).toBe(false);
    });

    it("keep a note only while it says something", () => {
        setOAuthNote("g1", { missingScopes: [CAL] });
        expect(oauthNote("g1")?.missingScopes).toEqual([CAL]);
        setOAuthNote("g1", { missingScopes: [] });
        expect(oauthNote("g1")).toBeUndefined();
    });

    it("hand a card's Reconnect to its details once", () => {
        requestReconnect("g1");
        expect(takeReconnectRequest("other")).toBe(false);
        expect(takeReconnectRequest("g1")).toBe(true);
        expect(takeReconnectRequest("g1")).toBe(false);
    });
});
