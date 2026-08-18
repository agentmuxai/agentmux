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

export const SkillApi = {
    SkillListCommand(
        client: RpcClient,
        data: { agent_id: string },
        opts?: RpcOpts,
    ): Promise<SkillListItem[]> {
        return client.rpcCall("skill.list", data, opts);
    },

    SkillGetCommand(
        client: RpcClient,
        data: { agent_id: string; id: string },
        opts?: RpcOpts,
    ): Promise<Skill | null> {
        return client.rpcCall("skill.get", data, opts);
    },

    SkillUpsertCommand(
        client: RpcClient,
        data: {
            agent_id: string;
            id?: string;
            name: string;
            trigger?: string;
            skill_type?: string;
            description?: string;
            content?: string;
        },
        opts?: RpcOpts,
    ): Promise<Skill> {
        return client.rpcCall("skill.upsert", data, opts);
    },

    SkillDeleteCommand(
        client: RpcClient,
        data: { agent_id: string; id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("skill.delete", data, opts);
    },

    SkillBindCommand(
        client: RpcClient,
        data: { agent_id: string; skill_id: string },
        opts?: RpcOpts,
    ): Promise<{ bound: boolean }> {
        return client.rpcCall("skill.bind", data, opts);
    },

    SkillUnbindCommand(
        client: RpcClient,
        data: { agent_id: string; skill_id: string },
        opts?: RpcOpts,
    ): Promise<{ unbound: boolean }> {
        return client.rpcCall("skill.unbind", data, opts);
    },

    // ── Armory catalog (global skills only, no agent_id) ────────────────────

    SkillCatalogListCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<SkillCatalogItem[]> {
        return client.rpcCall("skill.catalog.list", data, opts);
    },

    SkillCatalogUpsertCommand(
        client: RpcClient,
        data: {
            id?: string;
            name: string;
            trigger?: string;
            skill_type?: string;
            description?: string;
            content?: string;
        },
        opts?: RpcOpts,
    ): Promise<Skill> {
        return client.rpcCall("skill.catalog.upsert", data, opts);
    },

    SkillCatalogDeleteCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("skill.catalog.delete", data, opts);
    },

    // Catalog-tier sibling of SkillBindCommand — no agent_id/check_s1 gate,
    // since the Armory's connection is never agent-authenticated and can
    // never satisfy SkillBindCommand's check_s1. See
    // docs/reports/REPORT_ARMORY_SKILLS_MARKDOWN_AND_BIND_BUG_2026_07_27.md.
    SkillCatalogBindCommand(
        client: RpcClient,
        data: { agent_id: string; skill_id: string },
        opts?: RpcOpts,
    ): Promise<{ bound: boolean }> {
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
        data: { agent_id: string },
        opts?: RpcOpts,
    ): Promise<SkillListItem[]> {
        return client.rpcCall("skill.catalog.list_for_agent", data, opts);
    },

    SkillCatalogUnbindCommand(
        client: RpcClient,
        data: { agent_id: string; skill_id: string },
        opts?: RpcOpts,
    ): Promise<{ unbound: boolean }> {
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
        data: { bundle_id: string; skill_id: string },
        opts?: RpcOpts,
    ): Promise<{ bound: boolean }> {
        return client.rpcCall("skill.catalog.bind_to_bundle", data, opts);
    },

    SkillCatalogUnbindFromBundleCommand(
        client: RpcClient,
        data: { bundle_id: string; skill_id: string },
        opts?: RpcOpts,
    ): Promise<{ unbound: boolean }> {
        return client.rpcCall("skill.catalog.unbind_from_bundle", data, opts);
    },

    SkillCatalogListForBundleCommand(
        client: RpcClient,
        data: { bundle_id: string },
        opts?: RpcOpts,
    ): Promise<SkillBundleListItem[]> {
        return client.rpcCall("skill.catalog.list_for_bundle", data, opts);
    },

    // Creates a NEW, PRIVATE skill scoped directly to a bundle — see
    // McpApi.McpCatalogUpsertForBundleCommand's identical comment.
    SkillCatalogUpsertForBundleCommand(
        client: RpcClient,
        data: {
            bundle_id: string;
            id?: string;
            name: string;
            trigger?: string;
            skill_type?: string;
            description?: string;
            content?: string;
        },
        opts?: RpcOpts,
    ): Promise<Skill> {
        return client.rpcCall("skill.catalog.upsert_for_bundle", data, opts);
    },
};
