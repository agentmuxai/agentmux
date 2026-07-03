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
    ): Promise<Skill[]> {
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
    ): Promise<Skill[]> {
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
};
