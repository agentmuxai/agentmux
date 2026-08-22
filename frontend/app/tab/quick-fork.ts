// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Quick-fork a tab's active agent into a new tab, full independent
// identity, carrying conversation history —
// SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21.md Phase 2.

import { getLayoutModelForTabById } from "@/layout/index";
import { getBlockComponentModel, WOS } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WorkspaceService } from "@/app/store/services";
import { workspace } from "@/app/store/window-identity";
import { createBlockOnModel, resolveBlockDef, waitForLayoutModel } from "./tab-presets";
import { Logger } from "@/util/logger";

export interface ActiveAgentForTab {
    blockId: string;
    definitionId: string;
    sessionId: string;
}

/**
 * Resolve the agent currently active in a tab, from its focused layout
 * node's block meta. No dedicated "tab -> agent" index exists in this
 * codebase, so this composes the three pieces that do: `getLayoutModelForTabById`
 * (tab -> layout model), the focused leaf's `activeBlockId || blockId`
 * (layout node -> real block id, accounting for in-pane fork-bar
 * block-stacks — see `layoutNodeModels.ts`'s own comment on why
 * `activeBlockId` is "the field of intent"), and the block's own meta
 * (block -> agent definition id + live session id, both written by
 * `launchAgentDefinition` and kept current by the backend's
 * `persist_session_id` on every new CLI session capture, per
 * `agentmux-srv/src/backend/blockcontroller/core.rs`).
 *
 * Returns `null` for an empty tab, a non-agent-view block, or a tab with
 * nothing focused.
 */
export function resolveActiveAgentForTab(tabId: string): ActiveAgentForTab | null {
    const layoutModel = getLayoutModelForTabById(tabId);
    if (!layoutModel) return null;
    const node = layoutModel.focusedNode();
    if (!node) return null;
    const blockId: string | undefined = node.data?.activeBlockId || node.data?.blockId;
    if (!blockId) return null;
    const block = WOS.getObjectValue<Block>(WOS.makeORef("block", blockId));
    const meta = block?.meta;
    if (!meta || meta.view !== "agent" || !meta.agentId) return null;
    return {
        blockId,
        definitionId: meta.agentId as string,
        sessionId: (meta["agent:sessionid"] as string) ?? "",
    };
}

/**
 * Quick-fork the agent active in `sourceTabId` into a brand-new tab —
 * full independent identity (new `AgentDefinition`, never shares
 * `AGENTMUX_AGENT_ID` or a jekt signing key with the source, per
 * `template.rs`'s `forkagentdefinition` handler), conversation history
 * carried forward via `continueSessionId`/`forkSession` (Phase 1, PR
 * #2725), Armory/credential identity left **unbound by default** (spec
 * §5 — explicit opt-in to inherit the source's bound account is a later
 * phase, not this one).
 *
 * Reuses the SOURCE tab's own already-mounted `AgentViewModel` instance
 * to perform the launch (via the block-component registry,
 * `getBlockComponentModel` — the same lookup `refocusNode`/keybindings/
 * zoom handlers already use to reach a block's live view model from
 * outside its component tree) rather than constructing a new one.
 * `launchAgentDefinition`'s only two references to instance state are
 * `this.blockId` (already overridden by the explicit `targetBlockId`
 * argument here) and `this.nodejsError` (a minor, non-blocking rough
 * edge: a fork whose provider isn't installed would surface that error
 * on the SOURCE pane rather than the new one — acceptable, not a
 * correctness issue, and no worse than not surfacing it at all).
 *
 * The new tab's block is created via `waitForLayoutModel` +
 * `createBlockOnModel` (`tab-presets.ts` — the same path
 * `applyTabPreset` uses for every freshly-created tab), **not** a raw
 * `pane.open` with an explicit `tab_id`. Codex's review of PR #2727
 * caught that the two are NOT equivalent for a brand-new tab, and
 * `tab-presets.ts`'s own doc comment documents this as an empirically
 * confirmed gap, not a hypothetical one: a `pane.open` call against a
 * freshly created `tab_id` succeeds server-side with zero errors (block
 * created, layout updated) and STILL never renders, because the new
 * tab's client-side layout model isn't yet subscribed to receive the
 * backend's `layout:update` broadcast for that specific tab — even after
 * confirming the tab object itself exists. `createBlockOnModel` sidesteps
 * this entirely by mutating the local layout tree directly
 * (`layoutModel.treeReducer`), the same reactive path a normal `Cmd+T`
 * new tab already uses.
 *
 * @returns the new tab id on success, or `null` if the source tab has no
 *   active agent to fork, or any step failed. Errors are logged, not
 *   thrown — this is a fire-and-forget UI action, not something callers
 *   need to react to beyond "did it work."
 */
export async function quickForkTabToNewTab(sourceTabId: string): Promise<string | null> {
    const active = resolveActiveAgentForTab(sourceTabId);
    if (!active) {
        Logger.warn("quick-fork", "no active agent in source tab", { sourceTabId });
        return null;
    }

    const sourceBcm = getBlockComponentModel(active.blockId);
    const sourceModel = sourceBcm?.viewModel as unknown as
        | { launchAgentDefinition: (agent: any, overrides?: any, targetBlockId?: string, targetTabId?: string) => Promise<boolean> }
        | undefined;
    if (!sourceModel?.launchAgentDefinition) {
        Logger.warn("quick-fork", "source block has no live AgentViewModel", { blockId: active.blockId });
        return null;
    }

    const ws = workspace();
    if (!ws) {
        Logger.warn("quick-fork", "no current workspace to create the new tab in");
        return null;
    }

    try {
        const forkedDef = await RpcApi.ForkAgentDefinitionCommand(TabRpcClient, {
            source_id: active.definitionId,
            branch_label: "",
        });

        const newTabId = await WorkspaceService.CreateTab(ws.oid, forkedDef.name, true, false);

        const layoutModel = await waitForLayoutModel(newTabId);
        if (!layoutModel) {
            Logger.error("quick-fork", "new tab's layout model never became ready", { newTabId });
            return null;
        }
        const blockDef = resolveBlockDef("defwidget@agent");
        if (!blockDef) {
            Logger.error("quick-fork", "could not resolve the agent widget's blockdef");
            return null;
        }
        const newBlockId = await createBlockOnModel(newTabId, layoutModel, blockDef, null, null);

        const launched = await sourceModel.launchAgentDefinition(
            forkedDef,
            {
                instanceName: forkedDef.name,
                agentType: (forkedDef.agent_type as "host" | "container") || "host",
                environment: forkedDef.agent_type === "container" ? "docker" : "local",
                // Spec §5 decision: unbound by default, not the source's
                // bound account — a "quick" one-click action shouldn't
                // silently fan out credential access to a second agent.
                accountId: "",
                memoryId: "",
                continueSessionId: active.sessionId,
                forkSession: true,
            },
            newBlockId,
            newTabId
        );
        if (!launched) {
            Logger.warn("quick-fork", "launchAgentDefinition reported failure", { newTabId });
        }
        return newTabId;
    } catch (e: any) {
        Logger.error("quick-fork", "failed", { error: String(e) });
        return null;
    }
}
