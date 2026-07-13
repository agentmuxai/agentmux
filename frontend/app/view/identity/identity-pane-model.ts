// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Identity pane — the agent-settings `view: "identity"` tab's ViewModel.
//
// PR 5 of SPEC_BUNDLE_MANAGEMENT_2026_05_22.md demoted this tab to a
// read-only summary (see identity-pane-view.tsx), which originally read
// no state off this ViewModel. Phase 4b of
// SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md removed the Identity
// bundle CRUD/binding apparatus this class used to own — it fetched
// state via `refreshBundles()`/`refreshBindings()` that the view never
// rendered.
//
// Armory Phase 5 (docs/specs/SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md
// §1.3) closed the long-documented data gap: `agentId` below reads the
// same `meta.agentId` field the agent pane's own block already uses
// (see `AgentViewModel`/`agent-view.tsx`), so the view can resolve
// "this agent's" linked accounts when this pane's block happens to be
// opened with that meta set. When it isn't set (e.g. a generically
// opened identity block with no agent context), `agentId` is
// `undefined` and the view degrades to a context-free empty state.

import { BlockNodeModel } from "@/app/block/blocktypes";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { createMemo, type Accessor } from "solid-js";

export class IdentityPaneViewModel implements ViewModel {
    viewType = "identity";
    blockId: string;
    nodeModel: BlockNodeModel | null;

    viewIcon: Accessor<string> = () => "user";
    viewName: Accessor<string>;
    viewText: Accessor<string | HeaderElem[]> = () => "Identity";
    noPadding: Accessor<boolean> = () => false;

    get viewComponent(): ViewComponent {
        return null; // overridden by the barrel
    }

    blockAtom: Accessor<Block | undefined>;
    /** The specific agent this identity pane belongs to, if any (`meta.agentId`,
     *  same field `AgentViewModel` reads). `undefined` when this block was
     *  opened without agent context. */
    agentId: Accessor<string | undefined>;

    constructor(blockId?: string, nodeModel?: BlockNodeModel) {
        this.blockId = blockId ?? "";
        this.nodeModel = nodeModel ?? null;
        this.blockAtom = blockId
            ? getWaveObjectAtom(makeORef("block", blockId))
            : () => undefined;
        this.viewName = createMemo(() => {
            const block = this.blockAtom();
            return (block?.meta?.["frame:title"] as string) ?? "Identity";
        });
        this.agentId = createMemo(() => {
            const block = this.blockAtom();
            return block?.meta?.["agentId"] as string | undefined;
        });
    }
}
