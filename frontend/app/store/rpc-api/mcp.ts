// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// v1 composable model — standalone MCP Server primitive
// (agentmux-srv/src/server/app_api/mcp.rs). Agent-scoped: every command is
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
export type { McpAgentScopeData } from "@/types/rpc/McpAgentScopeData";
export type { McpAgentItemData } from "@/types/rpc/McpAgentItemData";
export type { McpAgentBindingData } from "@/types/rpc/McpAgentBindingData";
export type { McpBundleBindingData } from "@/types/rpc/McpBundleBindingData";
export type { McpBundleScopeData } from "@/types/rpc/McpBundleScopeData";
export type { McpCatalogItemData } from "@/types/rpc/McpCatalogItemData";
export type { McpCatalogListData } from "@/types/rpc/McpCatalogListData";
export type { CommandMcpUpsertData } from "@/types/rpc/CommandMcpUpsertData";
export type { CommandMcpCatalogUpsertData } from "@/types/rpc/CommandMcpCatalogUpsertData";
export type { CommandMcpCatalogUpsertForBundleData } from "@/types/rpc/CommandMcpCatalogUpsertForBundleData";
export type { McpDeleteResult } from "@/types/rpc/McpDeleteResult";
export type { McpBindResult } from "@/types/rpc/McpBindResult";
export type { McpUnbindResult } from "@/types/rpc/McpUnbindResult";

import type { McpAgentScopeData } from "@/types/rpc/McpAgentScopeData";
import type { McpAgentItemData } from "@/types/rpc/McpAgentItemData";
import type { McpAgentBindingData } from "@/types/rpc/McpAgentBindingData";
import type { McpBundleBindingData } from "@/types/rpc/McpBundleBindingData";
import type { McpBundleScopeData } from "@/types/rpc/McpBundleScopeData";
import type { McpCatalogItemData } from "@/types/rpc/McpCatalogItemData";
import type { McpCatalogListData } from "@/types/rpc/McpCatalogListData";
import type { CommandMcpUpsertData } from "@/types/rpc/CommandMcpUpsertData";
import type { CommandMcpCatalogUpsertData } from "@/types/rpc/CommandMcpCatalogUpsertData";
import type { CommandMcpCatalogUpsertForBundleData } from "@/types/rpc/CommandMcpCatalogUpsertForBundleData";
import type { McpDeleteResult } from "@/types/rpc/McpDeleteResult";
import type { McpBindResult } from "@/types/rpc/McpBindResult";
import type { McpUnbindResult } from "@/types/rpc/McpUnbindResult";

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
export type McpCatalogUpsertForBundleInput = Pick<
    CommandMcpCatalogUpsertForBundleData,
    "bundle_id" | "name"
> &
    Partial<Omit<CommandMcpCatalogUpsertForBundleData, "bundle_id" | "name">>;

export const McpApi = {
    McpListCommand(
        client: RpcClient,
        data: McpAgentScopeData,
        opts?: RpcOpts,
    ): Promise<McpServerListItem[]> {
        return client.rpcCall("mcp.list", data, opts);
    },

    McpGetCommand(
        client: RpcClient,
        data: McpAgentItemData,
        opts?: RpcOpts,
    ): Promise<McpServer | null> {
        return client.rpcCall("mcp.get", data, opts);
    },

    McpUpsertCommand(
        client: RpcClient,
        data: McpUpsertInput,
        opts?: RpcOpts,
    ): Promise<McpServer> {
        return client.rpcCall("mcp.upsert", data, opts);
    },

    McpDeleteCommand(
        client: RpcClient,
        data: McpAgentItemData,
        opts?: RpcOpts,
    ): Promise<McpDeleteResult> {
        return client.rpcCall("mcp.delete", data, opts);
    },

    McpBindCommand(
        client: RpcClient,
        data: McpAgentBindingData,
        opts?: RpcOpts,
    ): Promise<McpBindResult> {
        return client.rpcCall("mcp.bind", data, opts);
    },

    McpUnbindCommand(
        client: RpcClient,
        data: McpAgentBindingData,
        opts?: RpcOpts,
    ): Promise<McpUnbindResult> {
        return client.rpcCall("mcp.unbind", data, opts);
    },

    /** Health/prerequisite probe — see McpProbeResult (srv-types.d.ts). */
    McpProbeCommand(
        client: RpcClient,
        data: McpAgentItemData,
        opts?: RpcOpts,
    ): Promise<McpProbeResult> {
        return client.rpcCall("mcp.probe", data, opts);
    },

    // ── Armory catalog (global servers only, no agent_id) ──────────────────

    McpCatalogListCommand(
        client: RpcClient,
        data: McpCatalogListData = {},
        opts?: RpcOpts,
    ): Promise<McpServerCatalogItem[]> {
        return client.rpcCall("mcp.catalog.list", data, opts);
    },

    McpCatalogUpsertCommand(
        client: RpcClient,
        data: McpCatalogUpsertInput,
        opts?: RpcOpts,
    ): Promise<McpServer> {
        return client.rpcCall("mcp.catalog.upsert", data, opts);
    },

    McpCatalogDeleteCommand(
        client: RpcClient,
        data: McpCatalogItemData,
        opts?: RpcOpts,
    ): Promise<McpDeleteResult> {
        return client.rpcCall("mcp.catalog.delete", data, opts);
    },

    /** Health/prerequisite probe for a global catalog server — no agent_id
     *  (mirrors mcp.catalog.*'s window-scoped shape). See McpProbeResult. */
    McpCatalogProbeCommand(
        client: RpcClient,
        data: McpCatalogItemData,
        opts?: RpcOpts,
    ): Promise<McpProbeResult> {
        return client.rpcCall("mcp.catalog.probe", data, opts);
    },

    // Catalog-tier sibling of McpBindCommand — no agent_id/check_s1 gate,
    // since the Armory's connection is never agent-authenticated and can
    // never satisfy McpBindCommand's check_s1. See
    // docs/reports/REPORT_ARMORY_SKILLS_MARKDOWN_AND_BIND_BUG_2026_07_27.md.
    McpCatalogBindCommand(
        client: RpcClient,
        data: McpAgentBindingData,
        opts?: RpcOpts,
    ): Promise<McpBindResult> {
        return client.rpcCall("mcp.catalog.bind", data, opts);
    },

    // Catalog-tier siblings of McpListCommand / McpUnbindCommand — no
    // agent_id/check_s1 gate on the *caller*, but agent_id is still required
    // in the payload (whose bindings to list/unbind). Used by
    // AgentStashModal's MCP Servers tab, which runs over the dashboard's
    // connection and can never satisfy McpListCommand/McpUnbindCommand's
    // check_s1.
    McpCatalogListForAgentCommand(
        client: RpcClient,
        data: McpAgentScopeData,
        opts?: RpcOpts,
    ): Promise<McpServerListItem[]> {
        return client.rpcCall("mcp.catalog.list_for_agent", data, opts);
    },

    McpCatalogUnbindCommand(
        client: RpcClient,
        data: McpAgentBindingData,
        opts?: RpcOpts,
    ): Promise<McpUnbindResult> {
        return client.rpcCall("mcp.catalog.unbind", data, opts);
    },

    // ── Bundle-scoped siblings (composable model v2) ────────────────────
    // docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md, GH issue #2024
    // item 3. Same no-agent_id/no-check_s1 shape as the catalog trio above,
    // keyed by bundle_id instead of agent_id — only global servers (or ones
    // already bundle-bound) may be bound, same trust boundary as
    // McpCatalogBindCommand.

    McpCatalogBindToBundleCommand(
        client: RpcClient,
        data: McpBundleBindingData,
        opts?: RpcOpts,
    ): Promise<McpBindResult> {
        return client.rpcCall("mcp.catalog.bind_to_bundle", data, opts);
    },

    McpCatalogUnbindFromBundleCommand(
        client: RpcClient,
        data: McpBundleBindingData,
        opts?: RpcOpts,
    ): Promise<McpUnbindResult> {
        return client.rpcCall("mcp.catalog.unbind_from_bundle", data, opts);
    },

    McpCatalogListForBundleCommand(
        client: RpcClient,
        data: McpBundleScopeData,
        opts?: RpcOpts,
    ): Promise<McpServerBundleListItem[]> {
        return client.rpcCall("mcp.catalog.list_for_bundle", data, opts);
    },

    // Creates a NEW, PRIVATE server scoped directly to a bundle (never
    // global) — the actual "give this bundle its own tool" path.
    // McpCatalogBindToBundleCommand alone can only reference already-global
    // rows, which have no effect once bound (already unconditionally
    // visible to every agent).
    McpCatalogUpsertForBundleCommand(
        client: RpcClient,
        data: McpCatalogUpsertForBundleInput,
        opts?: RpcOpts,
    ): Promise<McpServer> {
        return client.rpcCall("mcp.catalog.upsert_for_bundle", data, opts);
    },
};
