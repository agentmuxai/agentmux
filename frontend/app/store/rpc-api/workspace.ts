// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Type re-exports for the connection, event, config and toolchain commands,
// kept so `@/app/store/rpc-api` still exports these names. Their stubs are
// generated (generated.ts).

// The toolchain wire shapes are GENERATED from their Rust definitions by
// ts-rs. `toolchain.env` and `toolchain.versions` built their responses from
// inline `json!({..})`, so neither end had a name to share.
export type { CheckCliAuthResult } from "@/types/rpc/CheckCliAuthResult";
export type { CommandCheckCliAuthData } from "@/types/rpc/CommandCheckCliAuthData";
export type { CommandResolveCliData } from "@/types/rpc/CommandResolveCliData";
export type { CommandRunCliLoginData } from "@/types/rpc/CommandRunCliLoginData";
export type { ResolveCliResult } from "@/types/rpc/ResolveCliResult";
export type { RunCliLoginResult } from "@/types/rpc/RunCliLoginResult";
export type { ToolchainEnvReq } from "@/types/rpc/ToolchainEnvReq";
export type { ToolchainEnvResult } from "@/types/rpc/ToolchainEnvResult";
export type { ToolchainPackage } from "@/types/rpc/ToolchainPackage";
export type { ToolchainPruneItem } from "@/types/rpc/ToolchainPruneItem";
export type { ToolchainPruneReq } from "@/types/rpc/ToolchainPruneReq";
export type { ToolchainPruneResult } from "@/types/rpc/ToolchainPruneResult";
export type { ToolchainPruneSkip } from "@/types/rpc/ToolchainPruneSkip";
export type { ToolchainVersionsReq } from "@/types/rpc/ToolchainVersionsReq";

// The connection-scoped event/config shapes are GENERATED from Rust by ts-rs.
export type { CommandEventReadHistoryData } from "@/types/rpc/CommandEventReadHistoryData";
export type { NoArgsReq } from "@/types/rpc/NoArgsReq";
export type { SubscriptionRequest } from "@/types/rpc/SubscriptionRequest";

// The tool-status shapes are GENERATED from their Rust definitions by ts-rs.
export type { CommandInstallToolData } from "@/types/rpc/CommandInstallToolData";
export type { GetToolStatusResult } from "@/types/rpc/GetToolStatusResult";
export type { InstallFailure } from "@/types/rpc/InstallFailure";
export type { InstallToolResult } from "@/types/rpc/InstallToolResult";
export type { ToolStatus } from "@/types/rpc/ToolStatus";
export type { ToolStatusEntry } from "@/types/rpc/ToolStatusEntry";
