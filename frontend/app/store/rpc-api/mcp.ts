// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// v1 composable model — standalone MCP Server primitive
// (crates/srv/src/server/app_api/mcp.rs). Agent-scoped: every command is
// `check_s1`-gated (ctx.agent_id must equal the request's agent_id), so
// these only work from an authenticated agent connection. The mcp.catalog.*
// commands are the window-scoped counterpart (no agent_id, global rows
// only) — that's what the Armory's MCP Servers tab uses.

import { RpcClient } from "../rpc-client";

// Every request and result shape below is GENERATED from its Rust definition by
// ts-rs. They were function-local anonymous `struct Req` declarations inside
// each handler closure -- seventeen of them -- so the inline types here were
// hand-maintained against types the frontend could not see.
//
// Deliberately NOT shared with the `skill` equivalents, even though the shapes
// rhyme: `{agent_id, mcp_id}` vs `{agent_id, skill_id}` differ in the field
// NAME, and collapsing them would mean renaming a wire field to serve an
// abstraction. That matches the 09-06 DRY audit's own recommendation on these
// twin primitives (cause 4).
export type { CommandMcpCatalogUpsertData } from "@/types/rpc/CommandMcpCatalogUpsertData";
export type { CommandMcpUpsertData } from "@/types/rpc/CommandMcpUpsertData";
export type { McpAgentBindingData } from "@/types/rpc/McpAgentBindingData";
export type { McpAgentItemData } from "@/types/rpc/McpAgentItemData";
export type { McpAgentScopeData } from "@/types/rpc/McpAgentScopeData";
export type { McpBindResult } from "@/types/rpc/McpBindResult";
export type { McpCatalogItemData } from "@/types/rpc/McpCatalogItemData";
export type { McpCatalogListData } from "@/types/rpc/McpCatalogListData";
export type { McpDeleteResult } from "@/types/rpc/McpDeleteResult";
export type { McpObservedAgent } from "@/types/rpc/McpObservedAgent";
export type { McpObservedServer } from "@/types/rpc/McpObservedServer";
export type { McpUnbindResult } from "@/types/rpc/McpUnbindResult";

import type { CommandMcpCatalogUpsertData } from "@/types/rpc/CommandMcpCatalogUpsertData";
import type { CommandMcpUpsertData } from "@/types/rpc/CommandMcpUpsertData";
import type { McpCatalogListData } from "@/types/rpc/McpCatalogListData";

// `id`, `transport` and `config` are all `#[serde(default)]` on the Rust side,
// so they are omittable on the wire, but ts-rs generates them as required (it
// cannot express an optional property for a non-`Option` field). Derive the
// accurate shape from the generated type rather than hand-listing the optional
// fields, so a field added in Rust flows through automatically. Same approach
// as `BundleUpsertInput` and `SkillUpsertInput`.
export type McpUpsertInput = Pick<CommandMcpUpsertData, "agent_id" | "name"> &
    Partial<Omit<CommandMcpUpsertData, "agent_id" | "name">>;
export type McpCatalogUpsertInput = Pick<CommandMcpCatalogUpsertData, "name"> &
    Partial<Omit<CommandMcpCatalogUpsertData, "name">>;

export const McpApi = {
    // ── Armory catalog (global servers only, no agent_id) ──────────────────

    McpCatalogListCommand(
        client: RpcClient,
        data: McpCatalogListData = {},
        opts?: RpcOpts
    ): Promise<McpServerCatalogItem[]> {
        return client.rpcCall("mcp.catalog.list", data, opts);
    },
};
