// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// "Bind to Agent" context menu for Armory account rows —
// SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md.
//
// The menu builder is a pure function (inputs → ContextMenuItem[]) so the
// filtering/annotation rules are unit-testable without a DOM or RPC mocks;
// the bind action itself lives here too so both the Armory tab and any
// future surface share one implementation.

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WOS } from "@/app/store/global";
import { getOpenDefinitionMap } from "@/app/store/agent-pane-state-store";
import { getProvider, resolveProviderAlias } from "@/app/view/agent/providers";
import { Logger } from "@/util/logger";
import type { Account } from "./identity-model";

/** One agent's row-worth of binding context for the submenu. */
export interface BindCandidate {
    agentId: string;
    agentName: string;
    /** Open pane blockId when the agent is currently running, else null. */
    runningBlockId: string | null;
    /** Bound to THIS account already. */
    boundHere: boolean;
    /** Name of the different account currently bound for this provider, if any. */
    boundElsewhereName: string | null;
}

/**
 * Compute the submenu candidates for binding `account` — the pure core.
 *
 * Rules (spec §3):
 *  - user-owned definitions only (`is_seeded === 0`);
 *  - CLI-OAuth accounts (claude/codex/…) only offer agents whose provider
 *    canonicalizes to the account's — alias-canonicalized on BOTH sides
 *    (the codex-P1-on-#2377 bug class);
 *  - service accounts (github/aws/… api-key class) offer every agent;
 *  - running agents (open pane) sort first, then by name.
 */
export function computeBindCandidates(
    account: Account,
    agents: AgentDefinition[],
    allLinks: AgentDefinitionIdentity[],
    openDefinitions: Map<string, string>,
    accountNameById: Map<string, string>,
): BindCandidate[] {
    const acctProvider = resolveProviderAlias(account.provider);
    // Spec §3's discriminator: CLI-OAuth accounts carry an oauth_config_dir
    // secret_ref (the CLI's per-account config dir). Service accounts
    // (github/aws/… api-key class) don't, and offer every agent.
    const cliOauth = account.secret_ref?.backend === "oauth_config_dir";

    const candidates: BindCandidate[] = [];
    for (const agent of agents) {
        if (agent.is_seeded !== 0) continue;
        if (cliOauth && resolveProviderAlias(agent.provider) !== acctProvider) continue;

        // This agent's current link for the account's provider (canonical
        // comparison — links can be stored under a legacy alias).
        const link = allLinks.find(
            (l) => l.agent_id === agent.id && resolveProviderAlias(l.provider) === acctProvider,
        );
        const boundHere = link?.account_id === account.id;
        const boundElsewhereName =
            link && !boundHere
                ? (accountNameById.get(link.account_id) ?? link.account_id)
                : null;

        candidates.push({
            agentId: agent.id,
            agentName: agent.name,
            runningBlockId: openDefinitions.get(agent.id) ?? null,
            boundHere,
            boundElsewhereName,
        });
    }

    candidates.sort((a, b) => {
        const runA = a.runningBlockId != null ? 0 : 1;
        const runB = b.runningBlockId != null ? 0 : 1;
        if (runA !== runB) return runA - runB;
        return a.agentName.localeCompare(b.agentName);
    });
    return candidates;
}

/** Sublabel for a candidate row — the live binding overview (spec §1). */
export function candidateSublabel(c: BindCandidate): string {
    const running = c.runningBlockId != null ? "● running" : "";
    const binding = c.boundHere
        ? "" // the checkmark carries this
        : c.boundElsewhereName != null
            ? `bound: ${c.boundElsewhereName}`
            : "no account bound";
    return [running, binding].filter(Boolean).join("  ·  ");
}

/**
 * Bind `account` to `candidate`'s agent (spec §2): link upsert, then — for
 * a running agent — the same `cmd:env` config-dir refresh
 * `useAgentControllerStatus.useGlobalLogin()` performs after linking, so
 * the new binding takes effect on the next turn without a restart (a stale
 * static `cmd:env` override would otherwise shadow the new link at the
 * next spawn).
 */
export async function bindAccountToAgent(account: Account, candidate: BindCandidate): Promise<void> {
    const provider = resolveProviderAlias(account.provider);
    await RpcApi.LinkAgentIdentityCommand(TabRpcClient, {
        agent_id: candidate.agentId,
        account_id: account.id,
        provider,
    });
    Logger.info(
        "identity",
        `armory bind: account ${account.id} (${account.name}) → agent ${candidate.agentId} (${candidate.agentName})`,
    );

    const blockId = candidate.runningBlockId;
    const dir = account.secret_ref?.dir;
    const envVar = getProvider(provider)?.authConfigDirEnvVar;
    if (!blockId || !dir || !envVar) return;
    try {
        const envMeta = WOS.getObjectValue<Block>(WOS.makeORef("block", blockId))?.meta?.["cmd:env"];
        const prevEnv: Record<string, string> = {};
        if (envMeta && typeof envMeta === "object") {
            for (const [k, v] of Object.entries(envMeta as Record<string, unknown>)) {
                if (typeof v === "string") prevEnv[k] = v;
            }
        }
        await RpcApi.SetMetaCommand(TabRpcClient, {
            oref: WOS.makeORef("block", blockId),
            meta: { "cmd:env": { ...prevEnv, [envVar]: dir } },
        });
    } catch (e: any) {
        // The link itself succeeded — the binding applies at the next
        // clean spawn even if the live env refresh failed. Log, don't throw.
        Logger.warn("identity", `armory bind: live cmd:env refresh failed: ${e?.message ?? e}`);
    }
}

/**
 * Assemble the full context-menu item list for an account row. Fetches the
 * link snapshot itself (one ListAllAgentIdentitiesCommand — the same call
 * the delete-disclosure in this tab already makes) and returns plain
 * ContextMenuItem[]s ready for ContextMenuModel.showContextMenu.
 */
export async function buildAccountRowMenu(
    account: Account,
    agents: AgentDefinition[],
    accounts: Account[],
    onBound?: () => void,
): Promise<ContextMenuItem[]> {
    let allLinks: AgentDefinitionIdentity[] = [];
    try {
        allLinks = (await RpcApi.ListAllAgentIdentitiesCommand(TabRpcClient)) ?? [];
    } catch {
        // Best-effort: without links the submenu still binds correctly —
        // it just can't annotate current bindings.
    }
    const accountNameById = new Map(accounts.map((a) => [a.id, a.name] as const));
    const candidates = computeBindCandidates(
        account,
        agents,
        allLinks,
        getOpenDefinitionMap(),
        accountNameById,
    );

    const bindItem: ContextMenuItem =
        candidates.length === 0
            ? {
                  // Disabled-with-reason beats disappearing (spec §3) —
                  // matches the generic pane menu's disabled-Copy precedent.
                  label: "Bind to Agent",
                  sublabel: "no compatible agents in this channel",
                  enabled: false,
              }
            : {
                  label: "Bind to Agent",
                  type: "submenu",
                  submenu: candidates.map((c) => ({
                      label: c.agentName,
                      type: "checkbox" as const,
                      checked: c.boundHere,
                      sublabel: candidateSublabel(c) || undefined,
                      click: () => {
                          void bindAccountToAgent(account, c).then(() => onBound?.());
                      },
                  })),
              };

    return [
        bindItem,
        { type: "separator" },
        {
            label: "Copy account ID",
            click: () => {
                void import("@/util/clipboard").then(({ writeText }) => writeText(account.id));
            },
        },
    ];
}
