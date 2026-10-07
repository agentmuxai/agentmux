// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Links into Remotes from another pane (the picker's "Manage remotes…",
// SPEC_REMOTES_PANE_2026_10_05.md §4.1): a Remotes tab in that same pane,
// or the one it already has, with the named host's row expanded.

import { MOS } from "@/app/store/global";
import { setBlockMeta } from "@/app/store/block-meta";
import { getLayoutModelForStaticTab } from "@/layout/lib/layoutModelHooks";
import { addWidgetAsPaneTab, effectiveStack, setActiveBlockInStack } from "@/layout/lib/layoutStack";
import { fireAndForget } from "@/util/util";
import { META_REMOTES_EXPAND } from "./remotes-sections";

/** Open Remotes in the pane holding `fromBlockId` (switching to its Remotes
 *  tab if it has one), with `connection`'s row expanded when given. */
export async function openRemotesInPane(fromBlockId: string, connection?: string): Promise<void> {
    const model = getLayoutModelForStaticTab();
    const node = model?.getNodeByBlockId(fromBlockId);
    if (!model || !node?.data) throw new Error("This pane isn't in the window tab on screen.");
    const expand = connection ? { [META_REMOTES_EXPAND]: connection } : {};
    const existing = effectiveStack(node.data).find(
        (id) => MOS.getObjectValue<Block>(MOS.makeORef("block", id))?.meta?.view === "remotes"
    );
    if (existing) {
        if (connection) await setBlockMeta(existing, expand as MetaType);
        setActiveBlockInStack(model, node.id, existing);
        return;
    }
    await addWidgetAsPaneTab(model, node.id, { meta: { view: "remotes", ...expand } });
}

/** The pane header's "Remote settings…" (§4.1), for a pane on a remote: Remotes
 *  in this pane with that host's row expanded. Nothing for a local pane. */
export function remoteSettingsMenuItems(blockId: string, connection: string | null | undefined): ContextMenuItem[] {
    const conn = connection?.trim();
    if (!conn || conn === "local") return [];
    return [
        { type: "separator" },
        { label: "Remote settings…", click: () => fireAndForget(() => openRemotesInPane(blockId, conn)) },
    ];
}
