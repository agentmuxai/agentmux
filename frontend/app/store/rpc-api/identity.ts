// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Identity accounts + bundles, Armory key/OAuth flows, agent-identity
// links, and the pre-launch OAuth / install / prereq flows. Split from the
// original rpc-api.ts.

import { RpcClient } from "../rpc-client";
import type { OAuthFlowStatus } from "./types";

export const IdentityApi = {
    // ── v6: identity / instance / fork ──────────────────────────────────────

    ListIdentityAccountsCommand(
        client: RpcClient,
        data: { provider?: string } = {},
        opts?: RpcOpts,
    ): Promise<IdentityAccount[]> {
        return client.rpcCall("listidentityaccounts", data, opts);
    },

    GetIdentityAccountCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<IdentityAccount> {
        return client.rpcCall("getidentityaccount", data, opts);
    },

    UpsertIdentityAccountCommand(
        client: RpcClient,
        data: Partial<IdentityAccount>,
        opts?: RpcOpts,
    ): Promise<IdentityAccount> {
        return client.rpcCall("upsertidentityaccount", data, opts);
    },

    DeleteIdentityAccountCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("deleteidentityaccount", data, opts);
    },

    // Armory: optionally validate (validate=true → single user-initiated
    // outbound probe) then store an API key in the OS keychain. The plaintext
    // is never returned; on success the response carries only the masked tail +
    // non-secret metadata. See SPEC_TRUST_CENTER_2026_06_15.md §5/§6.
    AccountKeyVerifyCommand(
        client: RpcClient,
        data: {
            provider: string;
            name: string;
            displayName?: string;
            kind?: string;
            apiKey: string;
            validate: boolean;
            accountId?: string;
            context?: Record<string, unknown>;
        },
        opts?: RpcOpts,
    ): Promise<{
        valid: boolean;
        error?: string;
        accountId?: string;
        maskedTail?: string;
        status?: string;
        metadata?: Record<string, unknown>;
    }> {
        return client.rpcCall("account.key.verify", data, opts);
    },

    // Armory service OAuth (SPEC_TRUST_CENTER §4.2/§12.1). Resolves the
    // provider's OAuth config (built-in public client id, or BYO clientId/secret),
    // spawns the flow (PKCE loopback or device), and returns a session id + the
    // initial status. A "not configured" / unknown-provider case comes back as a
    // clean `error` field (not an RPC failure) so the UI can surface it.
    AccountOAuthStartCommand(
        client: RpcClient,
        data: { provider: string; name: string; clientId?: string; clientSecret?: string },
        opts?: RpcOpts,
    ): Promise<{ sessionId?: string; status?: OAuthFlowStatus; error?: string }> {
        return client.rpcCall("account.oauth.start", data, opts);
    },

    AccountOAuthPollCommand(
        client: RpcClient,
        data: { sessionId: string },
        opts?: RpcOpts,
    ): Promise<OAuthFlowStatus> {
        return client.rpcCall("account.oauth.poll", data, opts);
    },

    AccountOAuthCancelCommand(
        client: RpcClient,
        data: { sessionId: string },
        opts?: RpcOpts,
    ): Promise<{ cancelled: boolean }> {
        return client.rpcCall("account.oauth.cancel", data, opts);
    },

    LinkAgentIdentityCommand(
        client: RpcClient,
        data: { agent_id: string; account_id: string; provider: string },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("linkagentidentity", data, opts);
    },

    UnlinkAgentIdentityCommand(
        client: RpcClient,
        data: { agent_id: string; provider: string },
        opts?: RpcOpts,
    ): Promise<{ unlinked: boolean }> {
        return client.rpcCall("unlinkagentidentity", data, opts);
    },

    ListAgentIdentitiesCommand(
        client: RpcClient,
        data: { agent_id: string },
        opts?: RpcOpts,
    ): Promise<AgentDefinitionIdentity[]> {
        return client.rpcCall("listagentidentities", data, opts);
    },

    // ────────────────────────────────────────────────────────────────────
    // v7 — Identity bundles
    // ────────────────────────────────────────────────────────────────────

    ListIdentityBundlesCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<IdentityBundle[]> {
        return client.rpcCall("listidentitybundles", data, opts);
    },

    GetIdentityBundleCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<IdentityBundle> {
        return client.rpcCall("getidentitybundle", data, opts);
    },

    UpsertIdentityBundleCommand(
        client: RpcClient,
        data: Partial<IdentityBundle>,
        opts?: RpcOpts,
    ): Promise<IdentityBundle> {
        return client.rpcCall("upsertidentitybundle", data, opts);
    },

    DeleteIdentityBundleCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("deleteidentitybundle", data, opts);
    },

    BindIdentityAccountCommand(
        client: RpcClient,
        data: { identity_id: string; provider: string; account_id: string },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("bindidentityaccount", data, opts);
    },

    UnbindIdentityAccountCommand(
        client: RpcClient,
        data: { identity_id: string; provider: string },
        opts?: RpcOpts,
    ): Promise<{ unbound: boolean }> {
        return client.rpcCall("unbindidentityaccount", data, opts);
    },

    ListIdentityBindingsCommand(
        client: RpcClient,
        data: { identity_id: string },
        opts?: RpcOpts,
    ): Promise<IdentityBinding[]> {
        return client.rpcCall("listidentitybindings", data, opts);
    },

    // ── Pre-launch OAuth (spec: SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md)

    AuthStartCommand(
        client: RpcClient,
        data: {
            providerId: string;
            intoBundleId?: string;
            /** Issue #1624 PR-C Part B — bypass the bundle system
             *  entirely; a successful auth persists a standalone
             *  IdentityAccount. Mutually exclusive with `intoBundleId`
             *  (never set both). Wire-additive only for now — not yet
             *  set by any caller (that's PR 3). */
            directAccount?: boolean;
            /** Direct-account reconnect: non-empty to refresh an
             *  already-linked account's tokens in place. Ignored unless
             *  `directAccount` is set. */
            existingAccountId?: string;
            cliPath: string;
            authLoginArgs: string[];
            authCheckArgs: string[];
            authEnv?: Record<string, string>;
            /** Spawn the login subprocess under a PTY (run_cli_login's
             *  PTY branch). Required for providers whose auth subcommand
             *  refuses to run without an interactive TTY (OpenClaw). */
            requiresTty?: boolean;
        },
        opts?: RpcOpts,
    ): Promise<{ sessionId: string; authUrl?: string }> {
        return client.rpcCall("auth.start", data, opts);
    },

    // command "auth.poll" — flattened `{ providerId, ...AuthSessionStatus }`
    AuthPollCommand(
        client: RpcClient,
        data: { sessionId: string },
        opts?: RpcOpts,
    ): Promise<AuthSessionStatus & { providerId: string }> {
        return client.rpcCall("auth.poll", data, opts);
    },

    AuthSubmitCallbackCommand(
        client: RpcClient,
        data: { sessionId: string; callbackUrl: string },
        opts?: RpcOpts,
    ): Promise<{ success: boolean; error?: string }> {
        return client.rpcCall("auth.submitcallback", data, opts);
    },

    AuthCancelCommand(
        client: RpcClient,
        data: { sessionId: string },
        opts?: RpcOpts,
    ): Promise<{ success: boolean; error?: string }> {
        return client.rpcCall("auth.cancel", data, opts);
    },

    // ── Agent install (SPEC_AGENT_INSTALL_STAGE_2026_05_17.md) ────────────

    // command "install.start" — begin install of a provider's CLI; the
    // backend npm-installs into the per-version cache and streams output
    // via `install_chunk` WPS events scoped to `install:<sessionId>`.
    InstallStartCommand(
        client: RpcClient,
        data: {
            providerId: string;
            cliCommand: string;
            npmPackage: string;
            pinnedVersion: string;
        },
        opts?: RpcOpts,
    ): Promise<{ sessionId: string }> {
        return client.rpcCall("install.start", data, opts);
    },

    // command "install.cancel" — abort an in-flight install and remove
    // the partial dir.
    InstallCancelCommand(
        client: RpcClient,
        data: { sessionId: string },
        opts?: RpcOpts,
    ): Promise<{ success: boolean; error?: string }> {
        return client.rpcCall("install.cancel", data, opts);
    },

    // command "install.check" — probe the per-version install dir to
    // decide whether the provider's CLI is already installed. Reads the
    // same path that `install.start` writes to, so the picker's
    // "show install modal?" decision matches the install location.
    InstallCheckCommand(
        client: RpcClient,
        data: { providerId: string; cliCommand: string },
        opts?: RpcOpts,
    ): Promise<{ installed: boolean }> {
        return client.rpcCall("install.check", data, opts);
    },

    // command "resolve.prereqs" — probe the system PATH for each
    // requested tool via where/which. Returns one PrereqResult per
    // input tool preserving order. Path-only — never executes the
    // tools. See SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18.md.
    ResolvePrereqsCommand(
        client: RpcClient,
        data: { tools: string[] },
        opts?: RpcOpts,
    ): Promise<{ results: Array<{ tool: string; found: boolean; path: string | null }> }> {
        return client.rpcCall("resolve.prereqs", data, opts);
    },

    AuthSubmitApiKeyCommand(
        client: RpcClient,
        data: {
            providerId: string;
            intoBundleId?: string;
            apiKey: string;
            accountName: string;
        },
        opts?: RpcOpts,
    ): Promise<{ success: boolean; bundleId?: string; error?: string }> {
        return client.rpcCall("auth.submitapikey", data, opts);
    },
};
