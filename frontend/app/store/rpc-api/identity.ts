// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Identity accounts, Armory key/OAuth flows, agent-identity links, and
// the pre-launch OAuth / install / prereq flows. Split from the
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
    ): Promise<{
        deleted: boolean;
        /** Agent (definition) ids whose `db_agent_identity_links` rows were
         *  cascaded by this delete. Any of them with a live process still
         *  holds the account's tokens until restarted — drives the Armory
         *  delete-time disclosure (SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4
         *  §4). Optional so older backends' `{ deleted }` shape stays
         *  assignable. */
        affectedAgents?: string[];
    }> {
        return client.rpcCall("deleteidentityaccount", data, opts);
    },

    // Armory: optionally validate (validate=true → single user-initiated
    // outbound probe) then store an API key in the OS keychain. The plaintext
    // is never returned; on success the response carries only the masked tail +
    // non-secret metadata. See specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §5/§6.
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
        data: {
            agent_id: string;
            provider: string;
            /** Skip the agentcredentials:revoked broadcast — for an alias
             *  migration (same credential staying bound under its canonical
             *  provider id), not a real unbind. Default false. */
            silent?: boolean;
        },
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

    // Every direct link across every agent — powers the Armory "Identities"
    // read-only rail (issue #1624 PR-C), which needs all agents' bindings
    // up front rather than one ListAgentIdentitiesCommand call per rail row.
    ListAllAgentIdentitiesCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<AgentDefinitionIdentity[]> {
        return client.rpcCall("listallagentidentities", data, opts);
    },

    // ── Pre-launch OAuth (spec: SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md)

    AuthStartCommand(
        client: RpcClient,
        data: {
            providerId: string;
            /** Vestigial — bundle mode was retired in Phase 4c of
             *  SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md. Kept on the
             *  wire shape only; never set by `AuthFlowController`. */
            intoBundleId?: string;
            /** Always sent as `true` by `AuthFlowController` (the sole
             *  caller) — a successful auth persists a standalone
             *  IdentityAccount. */
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

    // ── System-toolchain install (SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md) ──

    // command "toolchain.resolve_install_command" — read-only preview:
    // resolves what command WOULD run to install `toolId` (git/node/npm/
    // python) via the platform's own package manager (winget/brew/a
    // detected Linux package manager), without side effects. `available:
    // false` means no installable command exists on this platform/machine
    // (e.g. brew not installed on macOS, no known package manager found
    // on Linux) — callers must fall back to the existing link+copy-command
    // UI, never treat this as an error.
    ToolchainResolveInstallCommandCommand(
        client: RpcClient,
        data: { toolId: string },
        opts?: RpcOpts,
    ): Promise<
        | { available: false }
        | { available: true; program: string; args: string[]; needsElevation: boolean; commandPreview: string }
    > {
        return client.rpcCall("toolchain.resolve_install_command", data, opts);
    },

    // command "toolchain.install_system_tool" — spawns the resolved
    // install command and streams output via the SAME `install_chunk` WPS
    // event shape `install.start` uses, scoped `install:<sessionId>`.
    // `InstallCancelCommand` above already works unchanged for these
    // sessions (shared session registry) — no separate cancel command.
    ToolchainInstallSystemToolCommand(
        client: RpcClient,
        data: { toolId: string },
        opts?: RpcOpts,
    ): Promise<{ sessionId: string }> {
        return client.rpcCall("toolchain.install_system_tool", data, opts);
    },

    // command "identity.ensureaccountdir" — mints (or resolves, when
    // existingAccountId is set) a per-account isolated config dir without
    // spawning a CLI or an OAuth handshake. Used by the terminal-login tier,
    // which must know where to point the login BEFORE opening a terminal, so
    // the credential lands under a real account's own dir instead of the
    // shared/global one — "single point, not global",
    // PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md §7.
    EnsureAccountDirCommand(
        client: RpcClient,
        data: { providerId: string; existingAccountId?: string },
        opts?: RpcOpts,
    ): Promise<{ accountId: string; dir?: string }> {
        return client.rpcCall("identity.ensureaccountdir", data, opts);
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
