// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Agent's contribution to the shared pane-tab model (pane-tab-model.tsx):
 * the pill shows the agent's name and its provider's brand logo — the same
 * identity AgentViewModel.viewName/viewIcon give the pane — and renames the
 * agent definition on double-click.
 */

import { MOS } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { registerPaneTabDescriptor, type PaneTabIcon } from "@/element/pane-tab-model";
import { HISTORY_TAB_FOR_META_KEY, historyTabLabel } from "./open-history-tab";
import { PROVIDERS, resolveProviderAlias } from "./providers";

/** Same precedence as AgentViewModel.viewIcon; undefined falls through to
 *  the shared default (frame:icon, then the "agent" view icon). */
export function agentTabIcon(meta: MetaType | undefined): PaneTabIcon | undefined {
    const provider = meta?.["agentProvider"];
    if (typeof provider === "string" && provider.length > 0) {
        return { kind: "provider", provider: resolveProviderAlias(provider) };
    }
    const agentId = meta?.["agentId"];
    if (typeof agentId === "string" && PROVIDERS[agentId]) {
        return { kind: "provider", provider: agentId };
    }
    const icon = meta?.["agentIcon"];
    if (typeof icon === "string" && icon.length > 0) return { kind: "fa", name: icon };
    return undefined;
}

registerPaneTabDescriptor("agent", {
    label: ({ meta }) => {
        // A history reader carries its live sibling's agentName, so it has
        // to read distinctly — and name whose history it is.
        if (meta?.[HISTORY_TAB_FOR_META_KEY]) return historyTabLabel(meta?.["agentName"]);
        const name = meta?.["agentName"];
        // Repo-owner call (PR #3341): the unlaunched-picker fallback reads
        // "Agent", not "New Agent" — both fallbacks (pane title and tab)
        // must agree, and a launched agent still shows its real name.
        return typeof name === "string" && name.length > 0 ? name : "Agent";
    },
    icon: ({ meta }) => agentTabIcon(meta),
    renamer: ({ blockId, meta }) => {
        const definitionId = meta?.["agentId"] as string | undefined;
        // A blank picker tab has nothing to rename, and a history tab must
        // never rename the definition it shares with its live sibling.
        if (!definitionId || meta?.[HISTORY_TAB_FOR_META_KEY]) return undefined;
        return async (title: string) => {
            // The definition is the authoritative store (fork tabs, the
            // picker) — if this fails nothing has been written anywhere.
            await RpcApi.RenameAgentDefinitionTitleCommand(TabRpcClient, { id: definitionId, title });
            // Denormalized copy the pill and pane title read; it also catches
            // up on the agent's next launch, so a failure here is harmless.
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref: MOS.makeORef("block", blockId),
                meta: { agentName: title } as any,
            }).catch(() => {});
        };
    },
});
