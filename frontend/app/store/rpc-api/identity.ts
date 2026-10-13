// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Identity accounts, Armory key/OAuth flows, agent-identity links, and
// the pre-launch OAuth / install / prereq flows. Split from the
// original rpc-api file.

import { RpcClient } from "../rpc-client";

// The install.* and resolve.prereqs shapes are GENERATED from their Rust
// definitions by ts-rs. The rest of this file is still hand-written -- identity
// spans four handler files and is being migrated a file at a time.
export type { AccountOAuthCancelResult } from "@/types/rpc/AccountOAuthCancelResult";
export type { AckResp } from "@/types/rpc/AckResp";
export type { AgentIdentityLink } from "@/types/rpc/AgentIdentityLink";
export type { CancelProviderAuthReq } from "@/types/rpc/CancelProviderAuthReq";
export type { CommandGetIdentityAccountData } from "@/types/rpc/CommandGetIdentityAccountData";
export type { CommandLinkAgentIdentityData } from "@/types/rpc/CommandLinkAgentIdentityData";
export type { CommandListAgentIdentitiesData } from "@/types/rpc/CommandListAgentIdentitiesData";
export type { CommandListAllAgentIdentitiesData } from "@/types/rpc/CommandListAllAgentIdentitiesData";
export type { CommandListIdentityAccountsData } from "@/types/rpc/CommandListIdentityAccountsData";
export type { CommandUnlinkAgentIdentityData } from "@/types/rpc/CommandUnlinkAgentIdentityData";
export type { EnsureAccountDirReq } from "@/types/rpc/EnsureAccountDirReq";
export type { EnsureAccountDirResp } from "@/types/rpc/EnsureAccountDirResp";
export type { IdentityAccount } from "@/types/rpc/IdentityAccount";
export type { InstallCancelReq } from "@/types/rpc/InstallCancelReq";
export type { InstallCancelResult } from "@/types/rpc/InstallCancelResult";
export type { InstallCheckReq } from "@/types/rpc/InstallCheckReq";
export type { InstallCheckResult } from "@/types/rpc/InstallCheckResult";
export type { InstallStartReq } from "@/types/rpc/InstallStartReq";
export type { InstallStartResult } from "@/types/rpc/InstallStartResult";
export type { OAuthSessionReq } from "@/types/rpc/OAuthSessionReq";
export type { PollProviderAuthReq } from "@/types/rpc/PollProviderAuthReq";
export type { PrereqToolResolution } from "@/types/rpc/PrereqToolResolution";
export type { ResolvePrereqsReq } from "@/types/rpc/ResolvePrereqsReq";
export type { ResolvePrereqsResult } from "@/types/rpc/ResolvePrereqsResult";
export type { SecretRef } from "@/types/rpc/SecretRef";
export type { StartProviderAuthReq } from "@/types/rpc/StartProviderAuthReq";
export type { SubmitAuthCallbackReq } from "@/types/rpc/SubmitAuthCallbackReq";
export type { SubmitProviderApiKeyReq } from "@/types/rpc/SubmitProviderApiKeyReq";
export type { UnlinkAgentIdentityResult } from "@/types/rpc/UnlinkAgentIdentityResult";

import type { AgentIdentityLink } from "@/types/rpc/AgentIdentityLink";
import type { CommandListAllAgentIdentitiesData } from "@/types/rpc/CommandListAllAgentIdentitiesData";
import type { IdentityAccount } from "@/types/rpc/IdentityAccount";
import type { PollProviderAuthReq } from "@/types/rpc/PollProviderAuthReq";
import type { StartProviderAuthReq } from "@/types/rpc/StartProviderAuthReq";
import type { SubmitProviderApiKeyReq } from "@/types/rpc/SubmitProviderApiKeyReq";
import type { OAuthFlowStatus } from "./types";

// `directAccount`, `existingAccountId`, `authEnv` and `requiresTty` are all
// `#[serde(default)]` on non-`Option` Rust fields, so they are omittable on the
// wire but ts-rs generates them as required. Derive the accurate shape from the
// generated type rather than hand-listing them, so a field added in Rust flows
// through automatically -- same approach as BundleUpsertInput/SkillUpsertInput.
export type AuthStartInput = Pick<StartProviderAuthReq, "providerId" | "cliPath" | "authLoginArgs" | "authCheckArgs"> &
    Partial<Omit<StartProviderAuthReq, "providerId" | "cliPath" | "authLoginArgs" | "authCheckArgs">>;

// The frontend has always called the agent<->account link row
// `AgentDefinitionIdentity`; the Rust type is `AgentIdentityLink`. Same three
// fields, so this is a naming alias rather than a second declaration.
export type { AgentIdentityLink as AgentDefinitionIdentity } from "@/types/rpc/AgentIdentityLink";

export const IdentityApi = {
    // ── v6: identity / instance / fork ──────────────────────────────────────

    ListIdentityAccountsCommand(
        client: RpcClient,
        data: { provider?: string } = {},
        opts?: RpcOpts
    ): Promise<IdentityAccount[]> {
        return client.rpcCall("listidentityaccounts", data, opts);
    },

    UpsertIdentityAccountCommand(
        client: RpcClient,
        data: Partial<IdentityAccount>,
        opts?: RpcOpts
    ): Promise<IdentityAccount> {
        return client.rpcCall("upsertidentityaccount", data, opts);
    },

    DeleteIdentityAccountCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts
    ): Promise<{
        deleted: boolean;
        /** Agent (definition) ids whose `db_agent_identity_links` rows were
         *  cascaded by this delete. Any of them with a live process still
         *  holds the account's tokens until restarted — drives the Armory
         *  delete-time disclosure (SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4
         *  §4). Optional so older backends' `{ deleted }` shape stays
         *  assignable. */
        affectedAgents?: string[];
        /** What removing the saved login did; null when there was no row
         *  (SPEC_ARMORY_ACCOUNTS_DELETE_AND_INLINE_DETAIL_2026_10_04 §3.4).
         *  Optional so older backends' shape stays assignable. */
        cleanup?: {
            outcome: "removed" | "absent" | "skipped" | "failed" | "none";
            path?: string;
            detail?: string;
            historyKept?: boolean;
            historyPath?: string;
        } | null;
    }> {
        return client.rpcCall("deleteidentityaccount", data, opts);
    },

    // Armory: optionally validate (validate=true → single user-initiated
    // outbound probe) then store an API key in the OS keychain. The plaintext
    // is never returned; on success the response carries only the masked tail +
    // non-secret metadata. See docs/specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §5/§6.
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
        opts?: RpcOpts
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
        opts?: RpcOpts
    ): Promise<{ sessionId?: string; status?: OAuthFlowStatus; error?: string }> {
        return client.rpcCall("account.oauth.start", data, opts);
    },

    AccountOAuthPollCommand(client: RpcClient, data: { sessionId: string }, opts?: RpcOpts): Promise<OAuthFlowStatus> {
        return client.rpcCall("account.oauth.poll", data, opts);
    },

    // Every direct link across every agent — powers the Armory "Identities"
    // read-only rail (issue #1624 PR-C), which needs all agents' bindings
    // up front rather than one ListAgentIdentitiesCommand call per rail row.
    ListAllAgentIdentitiesCommand(
        client: RpcClient,
        data: CommandListAllAgentIdentitiesData = {},
        opts?: RpcOpts
    ): Promise<AgentIdentityLink[]> {
        return client.rpcCall("listallagentidentities", data, opts);
    },

    // ── Pre-launch OAuth (spec: SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md)

    AuthStartCommand(
        client: RpcClient,
        data: AuthStartInput,
        opts?: RpcOpts
    ): Promise<{ sessionId: string; authUrl?: string }> {
        return client.rpcCall("auth.start", data, opts);
    },

    // command "auth.poll" — flattened `{ providerId, ...AuthSessionStatus }`
    AuthPollCommand(
        client: RpcClient,
        data: PollProviderAuthReq,
        opts?: RpcOpts
    ): Promise<AuthSessionStatus & { providerId: string }> {
        return client.rpcCall("auth.poll", data, opts);
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
        opts?: RpcOpts
    ): Promise<
        | { available: false }
        | {
              available: true;
              program: string;
              args: string[];
              needsElevation: boolean;
              commandPreview: string;
              /** Version this exact command would install, queried live from
               *  the package manager's own catalog — `null`/absent when the
               *  query failed or isn't implemented for this platform's
               *  package manager. Never a hardcoded guess (see #2942). */
              resolvedVersion?: string | null;
          }
    > {
        return client.rpcCall("toolchain.resolve_install_command", data, opts);
    },

    // command "toolchain.install_system_tool" — spawns the resolved
    // install command and streams output via the SAME `install_chunk` MPS
    // event shape `install.start` uses, scoped `install:<sessionId>`.
    // `InstallCancelCommand` above already works unchanged for these
    // sessions (shared session registry) — no separate cancel command.
    ToolchainInstallSystemToolCommand(
        client: RpcClient,
        data: { toolId: string },
        opts?: RpcOpts
    ): Promise<{ sessionId: string }> {
        return client.rpcCall("toolchain.install_system_tool", data, opts);
    },

    // NOTE: the backend handler for this command currently returns `Err`
    // unconditionally -- API-key persistence is deferred ("bundle persistence
    // lands in PR C"). So the success shape below describes a response that
    // cannot occur yet, which is why it is NOT generated: there is no Rust
    // success type to generate it from. The REQUEST is generated and accurate.
    AuthSubmitApiKeyCommand(
        client: RpcClient,
        data: SubmitProviderApiKeyReq,
        opts?: RpcOpts
    ): Promise<{ success: boolean; bundleId?: string; error?: string }> {
        return client.rpcCall("auth.submitapikey", data, opts);
    },
};
