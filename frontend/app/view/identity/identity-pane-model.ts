// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Identity pane — the agent-settings `view: "identity"` tab's ViewModel.
//
// PR 5 of SPEC_BUNDLE_MANAGEMENT_2026_05_22.md demoted this tab to a
// read-only `<BundleSummaryPanel/>` (see identity-pane-view.tsx), which
// reads no state off this ViewModel. Phase 4b of
// SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md removed the Identity
// bundle CRUD/binding apparatus this class used to own — it fetched
// state via `refreshBundles()`/`refreshBindings()` that the view never
// rendered. What remains is the cosmetic shell BlockRegistry needs to
// back the `view: "identity"` pane.

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
    }
}
