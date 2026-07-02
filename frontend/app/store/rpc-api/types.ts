// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Types exported by the RPC API surface. Re-exported from the package index so
// existing `import { ..., type OAuthFlowStatus } from ".../rpc-api"` keeps working.

/**
 * Wire shape of a Armory service-OAuth flow status (account.oauth.*).
 * Mirrors `oauth_status_wire()` in agentmux-srv/src/server/agent_handlers.rs.
 *   pending       — flow starting up
 *   url-available — PKCE: open `authUrl` in the browser
 *   code-emitted  — device flow: show `userCode` + `verificationUri`
 *   success       — backend created the account (keychain-backed); `accountId`
 *   failed        — `error` describes why
 */
export type OAuthFlowStatus =
    | { status: "pending" }
    | { status: "url-available"; authUrl: string }
    | { status: "code-emitted"; userCode: string; verificationUri: string }
    | { status: "success"; accountId: string }
    | { status: "failed"; error: string };
