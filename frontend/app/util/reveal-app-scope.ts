// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What an `app` scope reveal does in the live app (`revealOrOpen`, docs/reports/
 * REPORT_ACTION_RESULTS_ALWAYS_VISIBLE_ACROSS_WINDOWS_2026_10_31.md §4): look for the pane in the app's
 * other windows, go to it and raise its window, and when the raise is not confirmed, say where the pane is
 * in a notice that stays until it is used: "Go there" tries again, "Show here" moves the pane to this
 * window (never for a terminal or an agent, which would cut the session off).
 */

import { setNotificationAction } from "@/app/notification/usenotification";
import { atoms, pushNotification, workspace } from "@/app/store/global";
import { ClientService, WorkspaceService } from "@/app/store/services";
import { holdsLiveProcess, paneIdentity } from "@/app/util/pane-identity";
import { loadObject, revealBlock, revealInOtherWindow } from "@/app/util/reveal-block";
import type { AppRevealDeps, GoResult, PaneElsewhere } from "@/app/util/reveal-or-open";

/** The pane's own name for the notice: its agent's or title, else the view. */
function paneName(block: Block): string {
    const meta = block.meta ?? {};
    const name = [meta["agentName"], meta["title"], meta["name"]].find((v) => typeof v === "string" && v.trim());
    return typeof name === "string" ? name.trim() : String(meta["view"] ?? "That pane");
}

/**
 * A pane of the same view and identity in a window other than this one. A view or pane without an identity
 * (the agent launcher, a browser tab on Personal) is looked for in this window only: "the Agent pane" is not
 * one thing to go to.
 */
export async function findPaneInOtherWindow(def: BlockDef): Promise<PaneElsewhere | null> {
    const view = def.meta?.["view"] as string | undefined;
    const wanted = paneIdentity(view, def.meta);
    if (!view || !wanted) return null;
    const here = workspace()?.oid;
    const client = await ClientService.GetClientData();
    for (const windowId of client?.windowids ?? []) {
        const win = await loadObject<MuxWindow>("window", windowId);
        if (!win?.workspaceid || win.workspaceid === here) continue;
        const ws = await loadObject<Workspace>("workspace", win.workspaceid);
        if (!ws) continue;
        for (const tabId of [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])]) {
            const tab = await loadObject<Tab>("tab", tabId);
            for (const blockId of tab?.blockids ?? []) {
                const block = await loadObject<Block>("block", blockId);
                if (!block || block.meta?.["view"] !== view || paneIdentity(view, block.meta) !== wanted) continue;
                return { blockId, windowId, tabId, workspaceId: ws.oid, name: paneName(block), liveProcess: holdsLiveProcess(view) };
            }
        }
    }
    return null;
}

/** Show the pane in the window that holds it and raise that window. */
export async function goToPaneElsewhere(found: PaneElsewhere): Promise<GoResult> {
    const r = await revealInOtherWindow(found.blockId);
    const shown = r.outcome === "revealed" ? "revealed" : r.outcome === "timeout" ? "timeout" : "failed";
    return { shown, reason: r.outcome === "not-found" ? "not-found" : r.reason, raise: r.raise };
}

/** Move the pane into this window's shown tab and show it there. */
async function showHere(found: PaneElsewhere): Promise<void> {
    const myWs = workspace()?.oid;
    const myTab = atoms.activeTabId();
    if (!myWs || !myTab) return;
    // RedockFloatingPane, as a drop of the pane from another window does: it queues the pane's leaf in the
    // destination tab and its removal from the source, which a plain block move leaves undrawn in both.
    await WorkspaceService.RedockFloatingPane(found.blockId, found.tabId, found.workspaceId, myTab, myWs);
    await revealBlock(found.blockId, { source: "widget", localOnly: true });
}

/** The notice for a pane whose window did not come forward. */
export function offerPaneElsewhere(found: PaneElsewhere, why: "refused" | "unknown"): void {
    const id = `reveal-elsewhere:${found.blockId}`;
    setNotificationAction(`${id}:go`, () => void goToPaneElsewhere(found));
    const actions: NotificationActionType[] = [{ label: "Go there", actionKey: `${id}:go` }];
    if (!found.liveProcess) {
        setNotificationAction(`${id}:show`, () => void showHere(found));
        actions.push({ label: "Show here", actionKey: `${id}:show` });
    }
    pushNotification({
        id,
        icon: "fa-window-restore",
        title: `${found.name} is open in another window`,
        message:
            why === "refused"
                ? "The system kept that window behind this one."
                : "AgentMux could not confirm that window came forward.",
        timestamp: new Date().toISOString(),
        type: "info",
        persistent: true,
        actions,
    });
}

export const appRevealDeps: AppRevealDeps = {
    findInOtherWindow: findPaneInOtherWindow,
    goTo: goToPaneElsewhere,
    offer: offerPaneElsewhere,
};
