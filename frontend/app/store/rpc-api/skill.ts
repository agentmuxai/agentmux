// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// v1 composable model — standalone Skill primitive
// (crates/srv/src/server/app_api/skill.rs). Agent-scoped: every command is
// `check_s1`-gated (ctx.agent_id must name the same agent as the request's
// agent_id), so these only work from an authenticated agent connection.
// Distinct from the legacy agent-scoped AgentSkill (`agent_skill_*` /
// `db_agent_skills`) — this is the v1 standalone primitive
// (`db_skills`). The skill.catalog.* commands are the window-scoped
// counterpart (no `check_s1`, global rows only) — that's what the Armory's
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
export type { CommandSkillCatalogUpsertData } from "@/types/rpc/CommandSkillCatalogUpsertData";
export type { CommandSkillCatalogUpsertForBundleData } from "@/types/rpc/CommandSkillCatalogUpsertForBundleData";
export type { CommandSkillUpsertData } from "@/types/rpc/CommandSkillUpsertData";
export type { SkillAgentBindingData } from "@/types/rpc/SkillAgentBindingData";
export type { SkillAgentItemData } from "@/types/rpc/SkillAgentItemData";
export type { SkillAgentScopeData } from "@/types/rpc/SkillAgentScopeData";
export type { SkillBindResult } from "@/types/rpc/SkillBindResult";
export type { SkillBundleBindingData } from "@/types/rpc/SkillBundleBindingData";
export type { SkillBundleScopeData } from "@/types/rpc/SkillBundleScopeData";
export type { SkillCatalogItemData } from "@/types/rpc/SkillCatalogItemData";
export type { SkillCatalogListData } from "@/types/rpc/SkillCatalogListData";
export type { SkillDeleteResult } from "@/types/rpc/SkillDeleteResult";
export type { SkillUnbindResult } from "@/types/rpc/SkillUnbindResult";

import type { CommandSkillCatalogUpsertData } from "@/types/rpc/CommandSkillCatalogUpsertData";
import type { CommandSkillCatalogUpsertForBundleData } from "@/types/rpc/CommandSkillCatalogUpsertForBundleData";
import type { CommandSkillUpsertData } from "@/types/rpc/CommandSkillUpsertData";
import type { SkillCatalogListData } from "@/types/rpc/SkillCatalogListData";

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
export type SkillCatalogUpsertForBundleInput = Pick<CommandSkillCatalogUpsertForBundleData, "bundle_id" | "name"> &
    Partial<Omit<CommandSkillCatalogUpsertForBundleData, "bundle_id" | "name">>;

export const SkillApi = {
    SkillUpsertCommand(client: RpcClient, data: SkillUpsertInput, opts?: RpcOpts): Promise<Skill> {
        return client.rpcCall("skill.upsert", data, opts);
    },

    // ── Armory catalog (global skills only, no agent_id) ────────────────────

    SkillCatalogListCommand(
        client: RpcClient,
        data: SkillCatalogListData = {},
        opts?: RpcOpts
    ): Promise<SkillCatalogItem[]> {
        return client.rpcCall("skill.catalog.list", data, opts);
    },

    SkillCatalogUpsertCommand(client: RpcClient, data: SkillCatalogUpsertInput, opts?: RpcOpts): Promise<Skill> {
        return client.rpcCall("skill.catalog.upsert", data, opts);
    },

    // Creates a NEW, PRIVATE skill scoped directly to a bundle (never
    // global) — the actual "give this bundle its own skill" path.
    // SkillCatalogBindToBundleCommand alone can only reference already-global
    // rows, which already reach every agent.
    SkillCatalogUpsertForBundleCommand(
        client: RpcClient,
        data: SkillCatalogUpsertForBundleInput,
        opts?: RpcOpts
    ): Promise<Skill> {
        return client.rpcCall("skill.catalog.upsert_for_bundle", data, opts);
    },
};
