// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Account service catalog — the brands shown as tiles in the Armory
 * Accounts gallery, and which auth paths each offers. Clicking a tile opens a
 * chooser of the brand's `authModes`; picking one opens the Add-account form
 * preset to that provider + the matching account kind.
 *
 * `keyKind` is the account kind to preset for the "key" path (the form's
 * Kind dropdown still lets the user refine it). The "oauth" path always presets
 * kind = "oauth". See docs/specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §3/§4/§12.3.
 */

import type { AccountKind, AccountProvider } from "@/app/view/identity/identity-model";

export type AuthMode = "oauth" | "key";

export interface ServiceTile {
    id: AccountProvider;
    displayName: string;
    /** Auth paths offered, in chooser display order. */
    authModes: AuthMode[];
    /** Account kind to preset when the user picks the "key" path. */
    keyKind: AccountKind;
    /** One-line descriptor under the tile name. */
    blurb?: string;
}

export const SERVICE_CATALOG: ServiceTile[] = [
    // AgentMux Cloud — the flagship brand. A singleton browser sign-in (Cognito
    // PKCE), not a per-credential OAuth/Key account: the gallery intercepts its
    // tile click to open a dedicated connect panel, so `authModes`/`keyKind`
    // below are inert placeholders kept only to satisfy the ServiceTile shape.
    { id: "agentmux", displayName: "AgentMux", authModes: ["oauth"], keyKind: "api_key", blurb: "Amux Cloud" },
    { id: "github", displayName: "GitHub", authModes: ["oauth", "key"], keyKind: "pat", blurb: "Repos, Actions, PRs" },
    { id: "google", displayName: "Google", authModes: ["oauth"], keyKind: "api_key", blurb: "Workspace, Cloud" },
    // AWS OAuth (IAM Identity Center / OIDC device) isn't wired in the backend
    // yet — key-only for v1 so the chooser never offers a dead-end OAuth path.
    { id: "aws", displayName: "AWS", authModes: ["key"], keyKind: "role", blurb: "IAM, deploy" },
    { id: "openai", displayName: "OpenAI", authModes: ["key"], keyKind: "api_key", blurb: "API key" },
    // "oauth" here is Claude's own in-app CLI login (spec
    // SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.3 surface 3), NOT the
    // service-OAuth scaffold (oauth-catalog.ts's OAUTH_SERVICES) the
    // github/google/slack tiles above use — Anthropic has no device/PKCE
    // endpoint an app-driven client could hit (auth-broker Phase D
    // decision), so the gallery intercepts this tile's "oauth" pick the
    // same way it intercepts the AgentMux tile above, opening
    // ClaudeLoginPanel instead of the generic OAuthConnectPanel form path.
    { id: "anthropic", displayName: "Anthropic", authModes: ["oauth", "key"], keyKind: "api_key", blurb: "Browser sign-in, or API key" },
    { id: "slack", displayName: "Slack", authModes: ["oauth"], keyKind: "api_key", blurb: "Messaging" },
    { id: "custom", displayName: "Custom", authModes: ["key"], keyKind: "api_key", blurb: "Any bearer token" },
];

export function modeLabel(mode: AuthMode): { title: string; sub: string } {
    return mode === "oauth"
        ? { title: "Connect with OAuth", sub: "Browser login — no key to manage" }
        : { title: "Add API key / token", sub: "Validated & stored in your OS keychain" };
}
