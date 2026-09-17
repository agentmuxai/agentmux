// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// v1 composable model — standalone Skill primitive
// (agentmux-srv/src/server/app_api/skill.rs). Agent-scoped: every command is
// `check_s1`-gated (ctx.agent_id must equal the request's agent_id), so
// these only work from an authenticated agent connection. Distinct from the
// legacy agent-scoped AgentSkill (`agent_skill_*` / `db_agent_skills`,
// `AgentSkillCard.tsx` et al.) — this is the v1 standalone primitive
// (`db_skills`). The skill.catalog.* commands are the window-scoped
// counterpart (no agent_id, global rows only) — that's what the Armory's
// Skills tab uses.

import { RpcClient } from "../rpc-client";

// Every request and result shape below is GENERATED from its Rust definition by
// ts-rs. They were function-local anonymous `struct Req` declarations inside
// each handler closure -- sixteen of them -- so the inline types here were
// hand-maintained against types the frontend could not see.
//
// Several commands share a type because they genuinely share a shape:
// `skill.bind` and `skill.catalog.unbind` both take `{agent_id, skill_id}` for
// the same reason. The shared names describe the shape rather than one command.
export type { SkillAgentScopeData } from "@/types/rpc/SkillAgentScopeData";
export type { SkillAgentItemData } from "@/types/rpc/SkillAgentItemData";
export type { SkillAgentBindingData } from "@/types/rpc/SkillAgentBindingData";
export type { SkillBundleBindingData } from "@/types/rpc/SkillBundleBindingData";
export type { SkillBundleScopeData } from "@/types/rpc/SkillBundleScopeData";
export type { SkillCatalogItemData } from "@/types/rpc/SkillCatalogItemData";
export type { SkillCatalogListData } from "@/types/rpc/SkillCatalogListData";
export type { CommandSkillUpsertData } from "@/types/rpc/CommandSkillUpsertData";
export type { CommandSkillCatalogUpsertData } from "@/types/rpc/CommandSkillCatalogUpsertData";
export type { CommandSkillCatalogUpsertForBundleData } from "@/types/rpc/CommandSkillCatalogUpsertForBundleData";
export type { SkillDeleteResult } from "@/types/rpc/SkillDeleteResult";
export type { SkillBindResult } from "@/types/rpc/SkillBindResult";
export type { SkillUnbindResult } from "@/types/rpc/SkillUnbindResult";

import type { SkillAgentScopeData } from "@/types/rpc/SkillAgentScopeData";
import type { SkillAgentItemData } from "@/types/rpc/SkillAgentItemData";
import type { SkillAgentBindingData } from "@/types/rpc/SkillAgentBindingData";
import type { SkillBundleBindingData } from "@/types/rpc/SkillBundleBindingData";
import type { SkillBundleScopeData } from "@/types/rpc/SkillBundleScopeData";
import type { SkillCatalogItemData } from "@/types/rpc/SkillCatalogItemData";
import type { SkillCatalogListData } from "@/types/rpc/SkillCatalogListData";
import type { CommandSkillUpsertData } from "@/types/rpc/CommandSkillUpsertData";
import type { CommandSkillCatalogUpsertData } from "@/types/rpc/CommandSkillCatalogUpsertData";
import type { CommandSkillCatalogUpsertForBundleData } from "@/types/rpc/CommandSkillCatalogUpsertForBundleData";
import type { SkillDeleteResult } from "@/types/rpc/SkillDeleteResult";
import type { SkillBindResult } from "@/types/rpc/SkillBindResult";
import type { SkillUnbindResult } from "@/types/rpc/SkillUnbindResult";

// The three upsert commands have every field except their scope key and `name`
// marked `#[serde(default)]`, so all of them are omittable on the wire. ts-rs
// generates them as REQUIRED because it cannot express an optional property for
// a non-`Option` field, so the accurate shape is DERIVED from the generated
// type rather than hand-listed -- a new field added in Rust flows through
// automatically and these cannot drift. Same approach as `BundleUpsertInput`.
export type SkillUpsertInput = Pick<CommandSkillUpsertData, "agent_id" | "name"> &
    Partial<Omit<CommandSkillUpsertData, "agent_id" | "name">>;
export type SkillCatalogUpsertInput = Pick<CommandSkillCatalogUpsertData, "name"> &
    Partial<Omit<CommandSkillCatalogUpsertData, "name">>;
export type SkillCatalogUpsertForBundleInput = Pick<
    CommandSkillCatalogUpsertForBundleData,
    "bundle_id" | "name"
> &
    Partial<Omit<CommandSkillCatalogUpsertForBundleData, "bundle_id" | "name">>;

export const SkillApi = {
    SkillListCommand(
        client: RpcClient,
        data: SkillAgentScopeData,
        opts?: RpcOpts,
    ): Promise<SkillListItem[]> {
        return client.rpcCall("skill.list", data, opts);
    },

    SkillGetCommand(
        client: RpcClient,
        data: SkillAgentItemData,
        opts?: RpcOpts,
    ): Promise<Skill | null> {
        return client.rpcCall("skill.get", data, opts);
    },

    SkillUpsertCommand(
        client: RpcClient,
        data: SkillUpsertInput,
        opts?: RpcOpts,
    ): Promise<Skill> {
        return client.rpcCall("skill.upsert", data, opts);
    },

    SkillDeleteCommand(
        client: RpcClient,
        data: SkillAgentItemData,
        opts?: RpcOpts,
    ): Promise<SkillDeleteResult> {
        return client.rpcCall("skill.delete", data, opts);
    },

    SkillBindCommand(
        client: RpcClient,
        data: SkillAgentBindingData,
        opts?: RpcOpts,
    ): Promise<SkillBindResult> {
        return client.rpcCall("skill.bind", data, opts);
    },

    SkillUnbindCommand(
        client: RpcClient,
        data: SkillAgentBindingData,
        opts?: RpcOpts,
    ): Promise<SkillUnbindResult> {
        return client.rpcCall("skill.unbind", data, opts);
    },

    // ── Armory catalog (global skills only, no agent_id) ────────────────────

    SkillCatalogListCommand(
        client: RpcClient,
        data: SkillCatalogListData = {},
        opts?: RpcOpts,
    ): Promise<SkillCatalogItem[]> {
        return client.rpcCall("skill.catalog.list", data, opts);
    },

    SkillCatalogUpsertCommand(
        client: RpcClient,
        data: SkillCatalogUpsertInput,
        opts?: RpcOpts,
    ): Promise<Skill> {
        return client.rpcCall("skill.catalog.upsert", data, opts);
    },

    SkillCatalogDeleteCommand(
        client: RpcClient,
        data: SkillCatalogItemData,
        opts?: RpcOpts,
    ): Promise<SkillDeleteResult> {
        return client.rpcCall("skill.catalog.delete", data, opts);
    },

    // Catalog-tier sibling of SkillBindCommand — no agent_id/check_s1 gate,
    // since the Armory's connection is never agent-authenticated and can
    // never satisfy SkillBindCommand's check_s1. See
    // docs/reports/REPORT_ARMORY_SKILLS_MARKDOWN_AND_BIND_BUG_2026_07_27.md.
    SkillCatalogBindCommand(
        client: RpcClient,
        data: SkillAgentBindingData,
        opts?: RpcOpts,
    ): Promise<SkillBindResult> {
        return client.rpcCall("skill.catalog.bind", data, opts);
    },

    // Catalog-tier siblings of SkillListCommand / SkillUnbindCommand — no
    // agent_id/check_s1 gate on the *caller*, but agent_id is still required
    // in the payload (whose bindings to list/unbind). Used by
    // AgentStashModal's Skills tab, which runs over the dashboard's
    // connection and can never satisfy SkillListCommand/SkillUnbindCommand's
    // check_s1.
    SkillCatalogListForAgentCommand(
        client: RpcClient,
        data: SkillAgentScopeData,
        opts?: RpcOpts,
    ): Promise<SkillListItem[]> {
        return client.rpcCall("skill.catalog.list_for_agent", data, opts);
    },

    SkillCatalogUnbindCommand(
        client: RpcClient,
        data: SkillAgentBindingData,
        opts?: RpcOpts,
    ): Promise<SkillUnbindResult> {
        return client.rpcCall("skill.catalog.unbind", data, opts);
    },

    // ── Bundle-scoped siblings (composable model v2) ────────────────────
    // docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md, GH issue #2024
    // item 3. Same no-agent_id/no-check_s1 shape as the catalog trio above,
    // keyed by bundle_id instead of agent_id — only global skills (or ones
    // already bundle-bound) may be bound, same trust boundary as
    // SkillCatalogBindCommand.

    SkillCatalogBindToBundleCommand(
        client: RpcClient,
        data: SkillBundleBindingData,
        opts?: RpcOpts,
    ): Promise<SkillBindResult> {
        return client.rpcCall("skill.catalog.bind_to_bundle", data, opts);
    },

    SkillCatalogUnbindFromBundleCommand(
        client: RpcClient,
        data: SkillBundleBindingData,
        opts?: RpcOpts,
    ): Promise<SkillUnbindResult> {
        return client.rpcCall("skill.catalog.unbind_from_bundle", data, opts);
    },

    SkillCatalogListForBundleCommand(
        client: RpcClient,
        data: SkillBundleScopeData,
        opts?: RpcOpts,
    ): Promise<SkillBundleListItem[]> {
        return client.rpcCall("skill.catalog.list_for_bundle", data, opts);
    },

    // Creates a NEW, PRIVATE skill scoped directly to a bundle — see
    // McpApi.McpCatalogUpsertForBundleCommand's identical comment.
    SkillCatalogUpsertForBundleCommand(
        client: RpcClient,
        data: SkillCatalogUpsertForBundleInput,
        opts?: RpcOpts,
    ): Promise<Skill> {
        return client.rpcCall("skill.catalog.upsert_for_bundle", data, opts);
    },
};
