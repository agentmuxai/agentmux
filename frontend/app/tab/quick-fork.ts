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
import { HISTORY_SOURCE_BLOCK_ID_META_KEY, HISTORY_TAB_FOR_META_KEY } from "@/app/view/agent/open-history-tab";
import { resolveEffectiveLaunchProvider } from "@/app/view/agent/agent-launch-env";
import { PROVIDERS, resolveProviderAlias } from "@/app/view/agent/providers";
import { Logger } from "@/util/logger";

/** Block-meta key the non-Claude fallback banner (`ForkProviderFallbackBanner`,
 *  `agent-view.tsx`) reads. Set once, after a fork lands, when the
 *  provider couldn't carry conversation history forward — see
 *  `quickForkTabToNewTab`'s doc comment. */
export const FORK_NO_HISTORY_FALLBACK_META_KEY = "quickfork:noHistoryFallback";

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
 * **Agent History readers are not live agents** (reagent's review of PR
 * #2727 caught this): `openOrFocusHistoryTab` creates a block with the
 * SAME `agentId` as the live agent it's a history view of, but it's
 * never actually launched — no `agent:sessionid` ever gets set on it.
 * `agent-model.ts` already excludes these (`meta[HISTORY_TAB_FOR_META_KEY]`)
 * when deciding whether an agent action applies; naively treating one as
 * "the active agent" here would resolve the right `definitionId` but an
 * empty `sessionId`, silently producing a no-history fork with no warning
 * — exactly the failure mode this feature exists to prevent. Rather than
 * just failing when the focused block turns out to be a history reader,
 * this falls back to the reader's own recorded source block
 * (`HISTORY_SOURCE_BLOCK_ID_META_KEY`, set at creation time) and resolves
 * *that* block's meta instead — a tab showing an agent's history still
 * has a real agent to fork, one hop away.
 *
 * Returns `null` for an empty tab, a non-agent-view block, a tab with
 * nothing focused, or a history reader whose recorded source block no
 * longer resolves to a live agent either.
 */
export function resolveActiveAgentForTab(tabId: string): ActiveAgentForTab | null {
    const layoutModel = getLayoutModelForTabById(tabId);
    if (!layoutModel) return null;
    const node = layoutModel.focusedNode();
    if (!node) return null;
    const blockId: string | undefined = node.data?.activeBlockId || node.data?.blockId;
    if (!blockId) return null;

    const resolved = resolveAgentBlock(blockId);
    if (!resolved) return null;
    if (resolved.meta[HISTORY_TAB_FOR_META_KEY]) {
        const sourceBlockId = resolved.meta[HISTORY_SOURCE_BLOCK_ID_META_KEY] as string | undefined;
        if (!sourceBlockId) return null;
        const source = resolveAgentBlock(sourceBlockId);
        if (!source) return null;
        return {
            blockId: sourceBlockId,
            definitionId: source.meta.agentId as string,
            sessionId: (source.meta["agent:sessionid"] as string) ?? "",
        };
    }
    return {
        blockId,
        definitionId: resolved.meta.agentId as string,
        sessionId: (resolved.meta["agent:sessionid"] as string) ?? "",
    };
}

function resolveAgentBlock(blockId: string): { meta: MetaType } | null {
    const block = WOS.getObjectValue<Block>(WOS.makeORef("block", blockId));
    const meta = block?.meta;
    if (!meta || meta.view !== "agent" || !meta.agentId) return null;
    return { meta };
}

/**
 * Quick-fork the agent active in `sourceTabId` into a brand-new tab —
 * full independent identity (new `AgentDefinition`, never shares
 * `AGENTMUX_AGENT_ID` or a jekt signing key with the source, per
 * `template.rs`'s `forkagentdefinition` handler), conversation history
 * carried forward via `continueSessionId`/`forkSession` (Phase 1, PR
 * #2725), Armory/credential identity left **unbound by default** (spec
 * §5) unless `opts.inheritIdentity` is explicitly set — Phase 4's
 * "confirm the identity choice" variant, sourced from the SOURCE
 * definition's own bound account via `ListAgentIdentitiesCommand`
 * (`db_agent_identity_links`, the same join `agent-identity-links-panel.tsx`
 * reads for the Identity tab), not a `RecentSessionRow` (this flow
 * resolves from block meta, not the picker's recent-sessions RPC).
 *
 * When the fork's effective provider (resolved through its bound bundle,
 * same as `launchAgentDefinition` itself does — `resolveEffectiveLaunchProvider`)
 * doesn't support `--fork-session`, `fork-session-args.ts`'s
 * `resolveForkSessionArgs` silently drops the session id inside
 * `launchAgentDefinition` rather than plain-resuming the parent's live
 * session (Codex's review of PR #2725). That's the right behavior, but
 * silent — per spec §4.4, the user needs a visible note that this
 * happened. Since there's no seam to push a message into the new block's
 * conversation before its own view even mounts, this sets
 * `FORK_NO_HISTORY_FALLBACK_META_KEY` on the new block's meta once launch
 * succeeds; `ForkProviderFallbackBanner` (`agent-view.tsx`) reads it.
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
export async function quickForkTabToNewTab(
    sourceTabId: string,
    opts?: { inheritIdentity?: boolean }
): Promise<string | null> {
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

        // Spec §5 decision: unbound by default, not the source's bound
        // account — a "quick" one-click action shouldn't silently fan out
        // credential access to a second agent. Phase 4: explicit opt-in
        // via opts.inheritIdentity looks up the SOURCE's own bound link
        // directly (this flow has a definitionId, not a RecentSessionRow).
        let accountId = "";
        if (opts?.inheritIdentity) {
            try {
                const links = await RpcApi.ListAgentIdentitiesCommand(TabRpcClient, {
                    agent_id: active.definitionId,
                });
                accountId = links[0]?.account_id ?? "";
            } catch (e: any) {
                Logger.warn("quick-fork", "failed to resolve source identity to inherit", { error: String(e) });
            }
        }

        // Non-Claude fallback note (spec §4.4) — resolved BEFORE launching
        // so it doesn't depend on launchAgentDefinition's return contract
        // (which is just a boolean). Only relevant when there was actually
        // a session to lose (an empty active.sessionId is already a fresh
        // start regardless of provider — nothing silently changed).
        let showNoHistoryFallback = false;
        if (active.sessionId) {
            // Same resolution as launchAgentDefinition itself (agent-model.ts):
            // effective provider -> PROVIDERS lookup, with an alias fallback.
            const effectiveProvider = await resolveEffectiveLaunchProvider(forkedDef);
            const provider = PROVIDERS[effectiveProvider] ?? PROVIDERS[resolveProviderAlias(effectiveProvider)];
            showNoHistoryFallback = provider?.id !== "claude";
        }

        const launched = await sourceModel.launchAgentDefinition(
            forkedDef,
            {
                instanceName: forkedDef.name,
                agentType: (forkedDef.agent_type as "host" | "container") || "host",
                environment: forkedDef.agent_type === "container" ? "docker" : "local",
                accountId,
                memoryId: "",
                continueSessionId: active.sessionId,
                forkSession: true,
            },
            newBlockId,
            newTabId
        );
        if (!launched) {
            Logger.warn("quick-fork", "launchAgentDefinition reported failure", { newTabId });
        } else if (showNoHistoryFallback) {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", newBlockId),
                meta: { [FORK_NO_HISTORY_FALLBACK_META_KEY]: true },
            }).catch((e: any) =>
                Logger.warn("quick-fork", "failed to set no-history-fallback meta", { error: String(e) })
            );
        }
        return newTabId;
    } catch (e: any) {
        Logger.error("quick-fork", "failed", { error: String(e) });
        return null;
    }
}
