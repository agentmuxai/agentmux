// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The first hooks of an agent pane: which agent and provider it is, whether it may render, its
 * modal opener, its model, the context meter and the shell drawer's log bridge. Split out of
 * AgentPresentationView; the hooks are called in the order they were there.
 */

import { usePaneTabVisibility } from "@/app/block/pane-tab-visibility";
import { ModalLayerContext } from "@/app/element/modal-layer";
import type { AgentDefinition } from "@/app/store/rpc-api";
import { createMemo, onCleanup, onMount, useContext } from "solid-js";
import { createAgentAtoms } from "../../agent-model/state";
import type { AgentViewModel } from "./agent-model";
import { useContextReading } from "./hooks/useContextReading";
import { usePaneRegistration } from "./hooks/usePaneRegistration";
import { useShellDrawerLifecycle } from "./hooks/useShellDrawerLifecycle";
import { useShellLogBridge } from "./hooks/useShellLogBridge";
import { useTurnLedger } from "./hooks/useTurnLedger";
import { paneMeta } from "./pane-meta";
import { getProvider } from "./providers";
import { useRenderDormant } from "./render-dormant";

export type PaneFoundation = ReturnType<typeof usePaneFoundation>;

export function usePaneFoundation(model: AgentViewModel, agentId: string, agentDefinitions: () => AgentDefinition[]) {
    const block = model.blockAtom;
    const meta = paneMeta(block, agentId);
    const { providerKey, outputFormat, agentName } = meta;
    // True while the user can't see this tab: hidden in the pane tab strip, or its window tab isn't displayed
    // (SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md). Render work pauses; the timed decisions (the question
    // timeout, the failure auto-retry) run on.
    const visibility = usePaneTabVisibility(model.blockId);
    const hidden = (): boolean => visibility() !== "active";
    const renderDormant = useRenderDormant(hidden, model.blockId); // `hidden`, or the window itself (minimized)
    const provider = () => getProvider(providerKey());
    // For the identity and memory modal requests.
    const currentAgent = createMemo(() => agentDefinitions().find((a) => a.id === agentId));

    // The right-click menu opens its modals in this pane's own layer (agent-body-menu.ts).
    const modalLayer = useContext(ModalLayerContext);
    onMount(() => {
        model._openModal = (req) => modalLayer?.open(req);
    });
    onCleanup(() => {
        model._openModal = null;
    });

    const agentAtoms = createMemo(() => createAgentAtoms());
    const { paneModel, layoutView } = usePaneRegistration({ blockId: model.blockId, agentId, agentName });

    // The context meter and its Swarm mirror (hooks/useContextReading.ts).
    const context = useContextReading(model.blockId, paneModel, block);
    // srv's turn ledger, so the working row covers the whole turn (hooks/useTurnLedger.ts).
    useTurnLedger(model.blockId, paneModel);

    // The shell drawer's log bridge: which log lines reach the shell terminal, and the backlog replayed when it mounts.
    const shell = useShellLogBridge(model.blockId);
    const { handleShellExited } = useShellDrawerLifecycle({
        blockId: model.blockId,
        shellSubBlockId: meta.shellSubBlockId,
        paneModel,
        clearTermWrite: shell.clearTermWrite,
    });

    return { block, meta, providerKey, outputFormat, agentName, renderDormant, provider, currentAgent, agentAtoms, paneModel, layoutView, context, shell, handleShellExited };
}
