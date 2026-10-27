// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The context menu for Connectors → Accounts rows, and `bindAccountToAgent`, the
// link-and-restart the agent pane's failure row still uses to adopt an account.
// The "Bind to Agent" submenu this file used to build is gone
// (SPEC_ACCOUNT_SWITCH_FROM_THE_COMPOSER_NO_BIND_2026_10_26.md).

import { RpcApi } from "@/app/store/rpc-api";
import { MENU_ICON } from "@/app/util/menu-icons";
import { TabRpcClient } from "@/app/store/rpc-util";
import { MOS, workspace } from "@/app/store/global";
import { getProvider, resolveProviderAlias } from "@/app/view/agent/providers";
import { Logger } from "@/util/logger";
import { notifyLinksChanged } from "./account-agents";
import type { Account } from "./identity-model";
import { mergeBlockEnv } from "@/app/store/block-meta";

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
    /** When the agent was last used (`lastUsedMs`), the order within each group. */
    lastUsedMs?: number;
}

/** Find the current workspace's tab holding `blockId`, for
 *  ControllerResyncCommand's required `tabid`. Same fast-path lookup
 *  swarm-view's focusBlock uses; returns null for a block in another
 *  window (resync is then skipped — see bindAccountToAgent). */
function findTabIdForBlock(blockId: string): string | null {
    const ws = workspace();
    if (!ws) return null;
    for (const tabId of [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])]) {
        const tab = MOS.getObjectValue<Tab>(MOS.makeORef("tab", tabId));
        if (tab?.blockids?.includes(blockId)) return tabId;
    }
    return null;
}

/**
 * Bind `account` to `candidate`'s agent (spec §2): link upsert, then — for
 * a running agent — the same live-apply pair the pane's own post-login
 * recovery performs after linking (this used to name
 * `useAgentControllerStatus.useGlobalLogin()`, removed 2026-08-31):
 * (1) refresh the block's `cmd:env` config-dir override (a stale static
 * override would otherwise shadow the new link at the next spawn), and
 * (2) `ControllerResyncCommand{forcerestart:true}` — a persistent
 * controller's already-alive CLI writes messages straight to the running
 * process's stdin and never re-reads env, so without the forced,
 * session-preserving respawn (`--resume`) the new binding would silently
 * not apply until a manual restart (reagentx P1 on #2485).
 *
 * The link upsert is the one step that must succeed — it throws on
 * failure so callers can surface it. The live-apply steps are best-effort
 * (logged, swallowed): the binding still applies at the next clean spawn.
 */
export async function bindAccountToAgent(account: Account, candidate: BindCandidate): Promise<void> {
    const provider = resolveProviderAlias(account.provider);
    try {
        // Alias-row cleanup BEFORE the upsert (reagentx P1 on #2485, second
        // round): the backend upserts on the RAW provider string
        // (ON CONFLICT(agent_id, provider)), so linking canonical "claude"
        // does not replace an existing link stored under a legacy alias
        // ("claude-code") — both rows survive, injection processes them
        // ORDER BY provider, and the alphabetically-later alias row applies
        // last and silently wins. Unlink every alias-stored row for this
        // canonical provider first (fetched fresh at click time — the
        // menu-build snapshot may be stale). `silent: true` is the flag
        // purpose-built for alias migration: the replacement link lands in
        // the same call, so a credentials-revoked broadcast would be noise.
        // Cleanup is part of the must-succeed path — proceeding past a
        // failed unlink would reintroduce the silent-wrong-account bug.
        const links = await RpcApi.ListAgentIdentitiesCommand(TabRpcClient, {
            agent_id: candidate.agentId,
        });
        for (const l of links ?? []) {
            if (l.provider !== provider && resolveProviderAlias(l.provider) === provider) {
                await RpcApi.UnlinkAgentIdentityCommand(TabRpcClient, {
                    agent_id: candidate.agentId,
                    provider: l.provider,
                    silent: true,
                });
            }
        }
        await RpcApi.LinkAgentIdentityCommand(TabRpcClient, {
            agent_id: candidate.agentId,
            account_id: account.id,
            provider,
        });
        notifyLinksChanged();
    } catch (e: any) {
        Logger.error(
            "identity",
            `bind to agent FAILED: account ${account.id} → agent ${candidate.agentId}: ${e?.message ?? e}`,
        );
        throw e;
    }
    Logger.info(
        "identity",
        `bind to agent: account ${account.id} (${account.name}) → agent ${candidate.agentId} (${candidate.agentName})`,
    );

    const blockId = candidate.runningBlockId;
    const dir = account.secret_ref?.dir;
    const envVar = getProvider(provider)?.authConfigDirEnvVar;
    if (!blockId || !dir || !envVar) return;
    try {
        await mergeBlockEnv(blockId, { [envVar]: dir });
        const tabId = findTabIdForBlock(blockId);
        if (tabId) {
            await RpcApi.ControllerResyncCommand(TabRpcClient, {
                tabid: tabId,
                blockid: blockId,
                forcerestart: true,
            });
        } else {
            Logger.warn(
                "identity",
                `bind to agent: pane ${blockId} not in this window's tabs — binding applies on its next spawn`,
            );
        }
    } catch (e: any) {
        // The link itself succeeded — the binding applies at the next
        // clean spawn even if the live apply failed. Log, don't throw.
        Logger.warn("identity", `bind to agent: live apply failed: ${e?.message ?? e}`);
    }
}

/**
 * The context-menu items for an account row. There is no "Bind to Agent" here:
 * an agent's account is changed from the login email in its composer
 * (SPEC_ACCOUNT_SWITCH_FROM_THE_COMPOSER_NO_BIND_2026_10_26.md).
 */
export function buildAccountRowMenu(
    account: Account,
    /** "Delete account…" — opens the shared delete confirmation
     *  (SPEC_ARMORY_ACCOUNTS_DELETE_AND_INLINE_DETAIL_2026_10_04.md §3.1).
     *  Omitted → no delete item. */
    onDelete?: () => void,
): ContextMenuItem[] {
    return [
        {
            label: "Copy account ID",
            icon: MENU_ICON.copy,
            click: () => {
                void import("@/util/clipboard").then(({ writeText }) => writeText(account.id));
            },
        },
        // Last, after a separator, so it is never the item under the
        // pointer when the menu opens.
        ...(onDelete
            ? ([{ type: "separator" }, { label: "Delete account…", icon: MENU_ICON.delete, click: onDelete }] as ContextMenuItem[])
            : []),
    ];
}
