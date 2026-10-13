// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Type re-exports for identity accounts, Armory key/OAuth flows,
// agent-identity links, and the pre-launch OAuth / install / prereq flows.
// Their stubs are generated (generated.ts); the shapes are GENERATED from
// their Rust definitions by ts-rs.
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
export type { UnlinkAgentIdentityResult } from "@/types/rpc/UnlinkAgentIdentityResult";

import type { StartProviderAuthReq } from "@/types/rpc/StartProviderAuthReq";

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
