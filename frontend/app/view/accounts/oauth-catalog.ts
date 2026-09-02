// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * OAuth service catalog (frontend) — which account providers can be connected
 * via the Armory's service-OAuth flow, and how. This is the frontend
 * view of the per-provider config in `agentmux-srv/src/identity/oauth_client.rs`
 * (`config_for`). See docs/specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §4.2/§12.1/§12.3.
 *
 * v1 wires GitHub (device flow) as the reference provider. The backend's
 * built-in public `client_id`s are not yet provisioned (`client_id: None`), so
 * `builtIn` is false for now — connecting uses the **BYO** path (the user
 * supplies their own OAuth app's client id). The moment a built-in client id is
 * baked into the backend catalog, flip `builtIn` to true here and the same UI
 * drives the zero-config built-in login.
 */

import type { AccountProvider } from "@/app/view/identity/identity-model";

export interface OAuthServiceInfo {
    provider: AccountProvider;
    /** Public-client flow this service uses (no confidential secret shipped). */
    flow: "device" | "pkce";
    /** Is a built-in public client id provisioned in the backend catalog today? */
    builtIn: boolean;
    /** Service mandates a confidential client secret (BYO must include one). */
    requiresSecret: boolean;
    /** Where the user registers their own OAuth app for the BYO path. */
    consoleUrl?: string;
    /** Short note on how to register the BYO app correctly. */
    byoHint?: string;
}

/**
 * Providers connectable via OAuth, keyed by `AccountProvider`. Only providers
 * present here show the "Connect with OAuth" option in the Accounts form.
 */
const OAUTH_SERVICES: Partial<Record<AccountProvider, OAuthServiceInfo>> = {
    github: {
        provider: "github",
        flow: "device",
        builtIn: false, // backend client_id not yet provisioned — BYO for now
        requiresSecret: false, // device flow needs no client secret
        consoleUrl: "https://github.com/settings/developers",
        byoHint:
            "Create a GitHub OAuth App (Settings → Developer settings → OAuth Apps), " +
            "enable “Device Flow”, and paste its Client ID below. No client secret is needed.",
    },
    google: {
        provider: "google",
        flow: "pkce",
        builtIn: false,
        requiresSecret: false, // PKCE loopback — no confidential secret
        consoleUrl: "https://console.cloud.google.com/apis/credentials",
        byoHint:
            "Create an OAuth client of type “Desktop app” in Google Cloud Console " +
            "and paste its Client ID below.",
    },
    slack: {
        provider: "slack",
        flow: "pkce",
        builtIn: false,
        requiresSecret: true, // Slack mandates a confidential client secret
        consoleUrl: "https://api.slack.com/apps",
        byoHint:
            "Create a Slack app, then paste its Client ID and Client Secret below " +
            "(Slack requires a client secret).",
    },
};

export function oauthInfo(provider: AccountProvider): OAuthServiceInfo | undefined {
    return OAUTH_SERVICES[provider];
}

export function supportsOAuth(provider: AccountProvider): boolean {
    return provider in OAUTH_SERVICES;
}

/** True when the connect flow must collect BYO client credentials from the user. */
export function needsByo(info: OAuthServiceInfo): boolean {
    return !info.builtIn || info.requiresSecret;
}
