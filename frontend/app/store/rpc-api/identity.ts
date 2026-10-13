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

import type { PollProviderAuthReq } from "@/types/rpc/PollProviderAuthReq";
import type { StartProviderAuthReq } from "@/types/rpc/StartProviderAuthReq";
import type { SubmitProviderApiKeyReq } from "@/types/rpc/SubmitProviderApiKeyReq";

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
