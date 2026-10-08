// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Links into Remotes from another pane (the picker's "Manage remotes…",
// SPEC_REMOTES_PANE_2026_10_05.md §4.1): a Connectors tab on its Remotes
// section in that same pane, or the one it already has, with the named host's
// row expanded (SPEC_REMOTES_INTO_CONNECTORS_2026_10_08.md §3.2).

import { MOS } from "@/app/store/global";
import { setBlockMeta } from "@/app/store/block-meta";
import { getLayoutModelForStaticTab } from "@/layout/lib/layoutModelHooks";
import { addWidgetAsPaneTab, effectiveStack, setActiveBlockInStack } from "@/layout/lib/layoutStack";
import { CONNECTORS_SECTION_KEY, CONNECTORS_VIEW, LEGACY_REMOTES_VIEW } from "@/app/view/section-pane/panes";
import { fireAndForget } from "@/util/util";
import { META_REMOTES_EXPAND } from "./remotes-sections";

/** Open Connectors → Remotes in the pane holding `fromBlockId` (switching to
 *  its Connectors tab if it has one), with `connection`'s row expanded when given. */
export async function openRemotesInPane(fromBlockId: string, connection?: string): Promise<void> {
    const model = getLayoutModelForStaticTab();
    const node = model?.getNodeByBlockId(fromBlockId);
    if (!model || !node?.data) throw new Error("This pane isn't in the window tab on screen.");
    const meta = {
        [CONNECTORS_SECTION_KEY]: "remotes" as const,
        ...(connection ? { [META_REMOTES_EXPAND]: connection } : {}),
    };
    // A saved Remotes tab not shown since it was a pane of its own still says
    // "remotes"; it becomes Connectors when it loads.
    const existing = effectiveStack(node.data).find((id) => {
        const view = MOS.getObjectValue<Block>(MOS.makeORef("block", id))?.meta?.view;
        return view === CONNECTORS_VIEW || view === LEGACY_REMOTES_VIEW;
    });
    if (existing) {
        await setBlockMeta(existing, meta as MetaType);
        setActiveBlockInStack(model, node.id, existing);
        return;
    }
    await addWidgetAsPaneTab(model, node.id, { meta: { view: CONNECTORS_VIEW, ...meta } });
}

/** The pane header's "Remote settings…" (§4.1), for a pane on a remote: Connectors
 *  → Remotes in this pane with that host's row expanded. Nothing for a local pane. */
export function remoteSettingsMenuItems(blockId: string, connection: string | null | undefined): ContextMenuItem[] {
    const conn = connection?.trim();
    if (!conn || conn === "local") return [];
    return [
        { type: "separator" },
        { label: "Remote settings…", click: () => fireAndForget(() => openRemotesInPane(blockId, conn)) },
    ];
}
