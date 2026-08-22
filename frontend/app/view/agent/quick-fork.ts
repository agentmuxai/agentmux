// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Quick-fork the agent live in a pane into a NEW pane-stack tab — a sibling
 * block pushed onto the SAME pane's own `blockStack`, right there in that
 * pane's tab strip next to "Agent History" (both trigger from
 * `AgentViewModel.getBodyContextMenuItems`, `agent-model.ts`) — full
 * independent identity, conversation history carried forward.
 *
 * This is the fork action `SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md`
 * §6.3 originally specced (the "`/btw` and the `+` affordance") but never
 * wired to a real trigger — `launchAgentDefinition`'s own `targetBlockId`
 * param existed for exactly this since 2026-07-20 (`agent-model.ts`'s doc
 * comment: "the fork-tab-strip `+` action") with no caller. A first
 * implementation attempt (`SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21.md`)
 * missed that pre-existing spec entirely and instead opened the fork in a
 * brand-new top-level WINDOW tab — corrected here per repo-owner feedback
 * back to the originally-specced destination: a pane-stack tab, using the
 * exact same `pane.open({skip_placement: true})` + `pushBlockOntoStack`
 * primitive `open-history-tab.ts`'s "Agent History" entry and the pane tab
 * strip's own "+" (`handleNewAgentTab`, `agent-view.tsx`) already use.
 *
 * `getLayoutModelForStaticTab()` reads a plain global atom, not SolidJS
 * component context, so — like `open-history-tab.ts` — this is safely
 * callable from anywhere, including a ViewModel method with no reactive-
 * owner scope of its own.
 */

import { closeBlockInStack, getLayoutModelForStaticTab, pushBlockOntoStack } from "@/layout/index";
import { atoms, pushNotification, WOS } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { ObjectService } from "@/app/store/services";
import { resolveEffectiveLaunchProvider } from "./agent-launch-env";
import { PROVIDERS, resolveProviderAlias } from "./providers";
import { lastLinkedAccountId } from "./providers/provider-id-aliases";
import type { LaunchOverrides } from "./components/AgentLaunchModal";
import { Logger } from "@/util/logger";

/** Block-meta key the non-Claude fallback banner (`ForkProviderFallbackBanner`,
 *  `agent-view.tsx`) reads. Set once, after a fork lands, when the
 *  provider couldn't carry conversation history forward — see
 *  `quickForkAgent`'s doc comment. */
export const FORK_NO_HISTORY_FALLBACK_META_KEY = "quickfork:noHistoryFallback";

/**
 * The minimal slice of `AgentViewModel` this needs. A structural type
 * rather than importing `AgentViewModel` itself — `agent-model.ts` is the
 * caller of this module, so importing its class type back here would be a
 * cycle — and it keeps this unit-testable against a plain mock object.
 */
export interface QuickForkModel {
    blockId: string;
    launchAgentDefinition: (
        agent: AgentDefinition,
        overrides?: LaunchOverrides,
        targetBlockId?: string,
        targetTabId?: string,
    ) => Promise<boolean>;
}

/**
 * Fork `model`'s own live agent into a new sibling tab in the SAME pane.
 * Full independent identity (new `AgentDefinition`, never shares
 * `AGENTMUX_AGENT_ID` or a jekt signing key with the source, per
 * `template.rs`'s `forkagentdefinition` handler), conversation history
 * carried forward via `continueSessionId`/`forkSession`, Armory/credential
 * identity left **unbound by default** (spec §5) unless `opts.inheritIdentity`
 * is explicitly set — sourced from the SOURCE definition's own bound account
 * via `ListAgentIdentitiesCommand`, filtered to the fork's own canonical
 * effective provider through `lastLinkedAccountId` (NOT a raw `.find()` —
 * `db_agent_identity_links` can hold both a canonical and a legacy-alias row
 * for the same provider at once; `lastLinkedAccountId`'s own doc comment
 * covers why the LAST such row, not the first, matches the real backend
 * spawn resolver).
 *
 * When the fork's effective provider (resolved through its bound bundle,
 * same as `launchAgentDefinition` itself does) doesn't support
 * `--fork-session`, `fork-session-args.ts`'s `resolveForkSessionArgs`
 * (invoked inside `launchAgentDefinition`) silently drops the session id
 * rather than plain-resuming the parent's live session. That's the right
 * behavior, but silent — this sets `FORK_NO_HISTORY_FALLBACK_META_KEY` on
 * the new block's meta once launch succeeds so `ForkProviderFallbackBanner`
 * can surface a visible note.
 *
 * @returns whether the fork actually launched. Never throws — failures are
 *   logged and surfaced via a toast, matching `openOrFocusHistoryTab`'s own
 *   self-contained-failure-handling convention (this has no wrapping caller
 *   of its own to push a notification on `false`).
 */
export async function quickForkAgent(
    model: QuickForkModel,
    opts?: { inheritIdentity?: boolean },
): Promise<boolean> {
    const meta = WOS.getObjectValue<Block>(WOS.makeORef("block", model.blockId))?.meta;
    const definitionId = meta?.["agentId"] as string | undefined;
    if (!definitionId) {
        Logger.warn("quick-fork", "pane has no live agent to fork", { blockId: model.blockId });
        return false;
    }
    const sessionId = (meta?.["agent:sessionid"] as string) ?? "";

    const layoutModel = getLayoutModelForStaticTab();
    const node = layoutModel.getNodeByBlockId(model.blockId);
    if (!node) return false;

    // Captured NOW, synchronously, before any `await` below — the pane
    // being right-clickable at all guarantees its own tab is the active
    // one at this exact instant. The several RPCs this function awaits
    // (fork, identity lookup, pane.open, launch) can take a while; if the
    // user switches window tabs mid-flight, `pane.open`'s `skip_placement`
    // path still resolves its OWN `tab_id` server-side ("explicit tab_id
    // wins, else split_reference_block_id's owner, else whichever tab is
    // globally active" — `open_pane`, agentmux-srv/src/server/app_api/mod.rs)
    // and `launchAgentDefinition`'s `ControllerResyncCommand` uses
    // `atoms.staticTabId()` (fixed at window bootstrap, not necessarily
    // this tab) when no override is given — either one would otherwise
    // silently register the new block under the WRONG tab (Codex's review
    // of this PR, two P1s). Passing this captured value through to both
    // closes that race.
    const ownerTabId = atoms.activeTabId();

    try {
        const forkedDef = await RpcApi.ForkAgentDefinitionCommand(TabRpcClient, {
            source_id: definitionId,
            branch_label: "",
        });

        // Same resolution as launchAgentDefinition itself (agent-model.ts):
        // effective provider -> PROVIDERS lookup, with an alias fallback.
        const effectiveProvider = await resolveEffectiveLaunchProvider(forkedDef);
        const canonicalForkProvider = resolveProviderAlias(effectiveProvider);
        const provider = PROVIDERS[effectiveProvider] ?? PROVIDERS[canonicalForkProvider];

        // Spec §5 decision: unbound by default, not the source's bound
        // account — a "quick" one-click action shouldn't silently fan out
        // credential access to a second agent. opts.inheritIdentity opts
        // in, looking up the SOURCE's own bound link directly.
        let accountId = "";
        if (opts?.inheritIdentity) {
            try {
                const links = await RpcApi.ListAgentIdentitiesCommand(TabRpcClient, {
                    agent_id: definitionId,
                });
                accountId = lastLinkedAccountId(links, provider?.id ?? canonicalForkProvider) ?? "";
            } catch (e: any) {
                Logger.warn("quick-fork", "failed to resolve source identity to inherit", { error: String(e) });
            }
        }

        // Non-Claude fallback note — only relevant when there was actually
        // a session to lose (an empty sessionId is already a fresh start
        // regardless of provider — nothing silently changed).
        const showNoHistoryFallback = !!sessionId && provider?.id !== "claude";

        // Allocate the new block WITHOUT placing it — same primitive
        // open-history-tab.ts / handleNewAgentTab (agent-view.tsx) use to
        // add a sibling into THIS pane's own stack.
        const paneOpenResult = (await TabRpcClient.rpcCall(
            "pane.open",
            { view: "agent", skip_placement: true, tab_id: ownerTabId, meta: { view: "agent" } },
            {},
        )) as { block_id: string };

        // The pane could have closed while the RPCs above were in flight —
        // re-resolve fresh rather than trusting the pre-await `node`
        // reference (same defensive check as open-history-tab.ts /
        // handleNewAgentTab).
        const freshNode = layoutModel.getNodeByBlockId(model.blockId);
        if (!freshNode) {
            await ObjectService.DeleteBlock(paneOpenResult.block_id).catch(() => {});
            return false;
        }
        pushBlockOntoStack(layoutModel, freshNode.id, paneOpenResult.block_id);

        const launched = await model.launchAgentDefinition(
            forkedDef,
            {
                instanceName: forkedDef.name,
                agentType: (forkedDef.agent_type as "host" | "container") || "host",
                environment: forkedDef.agent_type === "container" ? "docker" : "local",
                accountId,
                memoryId: "",
                continueSessionId: sessionId,
                forkSession: true,
            },
            paneOpenResult.block_id,
            ownerTabId,
        );
        if (!launched) {
            Logger.warn("quick-fork", "launchAgentDefinition reported failure", { blockId: paneOpenResult.block_id });
            // Don't leave the user on a blank/broken pane-stack tab — pop it
            // back out and delete the block, same as the "pane closed
            // mid-flight" cleanup above (Codex P2 on this PR).
            await closeBlockInStack(layoutModel, freshNode.id, paneOpenResult.block_id).catch((e: any) =>
                Logger.warn("quick-fork", "failed to clean up the failed fork's block", { error: String(e) }),
            );
        } else if (showNoHistoryFallback) {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", paneOpenResult.block_id),
                meta: { [FORK_NO_HISTORY_FALLBACK_META_KEY]: true },
            }).catch((e: any) =>
                Logger.warn("quick-fork", "failed to set no-history-fallback meta", { error: String(e) }),
            );
        }
        return launched;
    } catch (e: any) {
        Logger.error("quick-fork", "failed", { error: String(e) });
        pushNotification({
            icon: "fa-triangle-exclamation",
            title: "Quick-fork failed",
            message: e instanceof Error ? e.message : String(e),
            timestamp: new Date().toISOString(),
            type: "error",
            expiration: Date.now() + 8000,
        });
        return false;
    }
}
