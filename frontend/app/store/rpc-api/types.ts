// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Types exported by the RPC API surface. Re-exported from the package index so
// existing `import { ..., type OAuthFlowStatus } from ".../rpc-api"` keeps working.

/**
 * Wire shape of a service-OAuth flow status (account.oauth.*).
 * Mirrors `oauth_status_wire()` in
 * crates/srv/src/server/agent_handlers/identity.rs.
 *   pending       — flow starting up
 *   url-available — PKCE: open `authUrl` in the browser
 *   code-emitted  — device flow: show `userCode` + `verificationUri`
 *   success       — backend created or updated the account (keychain-backed);
 *                   `accountId`, the scopes the provider granted, the ones
 *                   asked for and unticked (a partial grant), and the email
 *                   from the ID token when the provider sends one
 *   failed        — `error` describes why
 *   confirm-user  — GitHub only: this AgentMux account is connected as another GitHub
 *                   user than the one who just authorized. The flow stopped; connect
 *                   again with `replaceGithubUser` once the person agrees.
 */
export type OAuthFlowStatus =
    | { status: "pending" }
    | { status: "url-available"; authUrl: string }
    | { status: "code-emitted"; userCode: string; verificationUri: string }
    | {
          status: "success";
          accountId: string;
          grantedScopes?: string[];
          missingScopes?: string[];
          email?: string | null;
      }
    | { status: "failed"; error: string }
    | { status: "confirm-user"; currentLogin: string; currentId: number; newLogin: string; newId: number };
