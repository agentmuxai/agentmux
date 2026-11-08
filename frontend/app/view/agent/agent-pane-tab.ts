// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Agent's contribution to the shared pane-tab model (pane-tab-model.tsx):
 * the pill shows the agent's name and its provider's brand logo — the same
 * identity AgentViewModel.viewName/viewIcon give the pane — and renames the
 * agent definition on double-click.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { PaneTabDescriptor, PaneTabIcon } from "@/element/pane-tab-model";
import { HISTORY_TAB_FOR_META_KEY, historyTabLabel } from "./open-history-tab";
import { dualLogoTitle } from "@/element/DualProviderLogo";
import { paneIconInput } from "./pane-icon-input";
import { setBlockMeta } from "@/app/store/block-meta";

/** Same precedence as AgentViewModel.viewIcon; undefined falls through to
 *  the shared default (frame:icon, then the "agent" view icon). */
export function agentTabIcon(meta: MetaType | undefined): PaneTabIcon | undefined {
    // The harness, with the model answering as its badge. The pill has no agent definitions at hand, so a
    // custom endpoint's own badge ("custom") is the header icon's, not the pill's.
    const shown = paneIconInput(meta, []);
    if (shown) return { kind: "provider", provider: shown.harness, vendor: shown.vendor };
    const icon = meta?.["agentIcon"];
    if (typeof icon === "string" && icon.length > 0) return { kind: "fa", name: icon };
    return undefined;
}

/** The agent manifest's `tab` (block-registry.ts). */
export const agentPaneTab: PaneTabDescriptor = {
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
    modelLine: ({ meta }) => {
        const shown = paneIconInput(meta, []);
        return shown ? dualLogoTitle(shown.harness, shown.vendor, shown.modelText) : undefined;
    },
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
            // up on the agent's next launch, so a failure here is harmless. A
            // pane title set earlier (`frame:title`, which outranks the name)
            // is cleared, so the pane shows the name it was just given.
            await setBlockMeta(blockId, { agentName: title, "frame:title": null }).catch(() => {});
        };
    },
};
