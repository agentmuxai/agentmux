// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Block } from "@/app/block/block";
import { PaneLeafChrome } from "@/app/tab/pane-leaf-chrome";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { ModalLayer } from "@/element/ModalLayer";
import { CenteredDiv } from "@/element/quickelems";
import logoUrl from "@/app/asset/logo-brain.svg?url";
import { ContentRenderer, NodeModel, PreviewRenderer, TileLayout } from "@/layout/index";
import { effectiveStack } from "@/layout/lib/layoutStack";
import { TileLayoutContents } from "@/layout/lib/types";
import { atoms, createBlock, getApi, getHostName, getUserName, isDev } from "@/store/global";
import * as services from "@/store/services";
import * as MOS from "@/store/mos";
import { buildPaneWidgetMenuItems } from "@/app/window/action-widgets-config";
import { ConfirmModal } from "@/app/element/confirm-modal";
import {
    busyMembers,
    closesWithShutdownLog,
    describeBusyMember,
    type BusyMember,
    type PaneCloseProbe,
} from "@/app/tab/pane-close-guard";
import { beginShutdownLog, endShutdownLog, failShutdownLog } from "@/app/view/agent/shutdown/shutdown-log";
import { removeMovedBlock } from "@/layout/lib/layoutMagnify";
import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import { pushFlashError } from "@/app/store/flash-notifications";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import type { JSX } from "solid-js";

/**
 * Build a widget menu for right-clicking an empty tab (no panes). Grouped
 * widgets (e.g. Messengers' Discord/Slack/etc.) nest under their parent's
 * own label instead of each showing up individually — see
 * buildPaneWidgetMenuItems.
 */
function buildEmptyTabMenu(): ContextMenuItem[] {
    const fullConfig = atoms.fullConfigAtom();
    const wmap = fullConfig?.widgets ?? {};
    const settings = fullConfig?.settings ?? {};
    return buildPaneWidgetMenuItems(wmap, settings, (blockdef) => void createBlock(blockdef));
}

/** Live probes for the pane-close confirmation (`pane-close-guard.ts`). */
const paneCloseProbe: PaneCloseProbe = {
    turnActive: async (blockId) => (await services.BlockService.GetControllerStatus(blockId))?.turn_active ?? null,
    processCount: async (blockId) =>
        (await RpcApi.AgentProcessListCommand(TabRpcClient, { block_id: blockId })).processes.length,
    name: (blockId) => {
        const meta = MOS.getObjectValue<MuxObj>(MOS.makeORef("block", blockId))?.meta;
        return (meta?.["agentName"] as string) || (meta?.["agentId"] as string) || "An agent";
    },
};

function isAgentBlock(blockId: string): boolean {
    return MOS.getObjectValue<MuxObj>(MOS.makeORef("block", blockId))?.meta?.["view"] === "agent";
}

/**
 * Close agent panes in place (§5.5): the pane stays, covered by its shutdown
 * log, and srv removes each tab from the layout once it is down — the
 * `delete` actions it queues for a waiting frontend. srv reports failures
 * during the close in the log itself; a rejection before it started is put
 * there too, so the pane offers Keep open / Try again instead of hanging.
 */
function closeWithShutdownLog(tabId: string, blockIds: string[]): void {
    const agentIds = blockIds.filter(isAgentBlock);
    for (const id of agentIds) beginShutdownLog(id);
    services.ObjectService.ClosePane(blockIds, true).catch((err) => {
        if (/block not found/i.test(String(err))) {
            // Every block was already gone, so srv can't name their tab to
            // queue the layout change: drop them here instead (a safe no-op
            // for any the layout no longer has).
            const model = getLayoutModelForTabById(tabId);
            for (const id of blockIds) {
                if (model) removeMovedBlock(model, id);
                endShutdownLog(id);
            }
            return;
        }
        for (const id of agentIds) failShutdownLog(id, String(err));
        pushFlashError({
            id: "",
            icon: "triangle-exclamation",
            title: "Couldn't close the pane",
            message: String(err),
            expiration: Date.now() + 10_000,
        });
    });
}

function TabContent(props: { tabId: string }): JSX.Element {
    const oref = createMemo(() => MOS.makeORef("tab", props.tabId));
    const tabAtom = createMemo(() => MOS.getMuxObjectAtom<Tab>(oref()));
    const tabData = createMemo(() => tabAtom()());

    const tileGapSize = createMemo(() => {
        const settings = atoms.settingsAtom();
        return settings["window:tilegapsize"];
    });

    // Queue, not a single slot: two busy panes closed in quick succession
    // each get their own answer, in order. A single slot let the second
    // prompt overwrite the first, whose close then waited forever
    // (reagent P1 on #3422).
    const [pendingCloses, setPendingCloses] = createSignal<
        { busy: BusyMember[]; resolve: (close: boolean) => void }[]
    >([]);
    const pendingClose = () => pendingCloses()[0] ?? null;
    const answerPendingClose = (close: boolean) => {
        const [head, ...rest] = pendingCloses();
        setPendingCloses(rest);
        head?.resolve(close);
    };
    // A tab that goes away with prompts still open cancels them, rather than
    // leaving their closes waiting forever.
    onCleanup(() => {
        for (const pending of pendingCloses()) pending.resolve(false);
    });

    const tileLayoutContents = createMemo<TileLayoutContents>(() => {
        const renderContent: ContentRenderer = (nodeModel: NodeModel) => {
            return <PaneLeafChrome nodeModel={nodeModel} />;
        };

        // Deliberately plain <Block>, not <PaneLeafChrome> — a drag-preview
        // thumbnail has no interactive tab strip to hoist (it's a static
        // snapshot, never switched), so routing it through the chrome
        // decision would be pure overhead for no behavior change.
        const renderPreview: PreviewRenderer = (nodeModel: NodeModel) => {
            return <Block nodeModel={nodeModel} preview={true} />;
        };

        // Every block in the closed leaf, not just the visible one — closing a
        // pane that holds several tabs used to leave every background tab's
        // agent running with no pane. `closeBlockInStack` passes `{ blockId }`
        // alone, which the backend treats as one tab of a surviving pane.
        // SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md §4.1.
        async function onNodeDelete(data: TabLayoutData) {
            const blockIds = effectiveStack(data).filter(Boolean);
            getApi().sendLog(`[BUG-TRACE] onNodeDelete ENTER for blockIds: ${blockIds.join(",")}`);
            try {
                const result = await services.ObjectService.ClosePane(blockIds);
                getApi().sendLog(`[BUG-TRACE] onNodeDelete ClosePane returned: ${JSON.stringify(result)}`);
                return result;
            } catch (err) {
                getApi().sendLog(`[BUG-TRACE] onNodeDelete ERROR: ${err}`);
                // The pane is already gone from the UI; the close runs through
                // `fireAndForget`, which would only log this. Tell the user —
                // an agent may still be running (spec §4.5). The orphan reaper
                // finishes the job if it can't be retried from here.
                pushFlashError({
                    id: "",
                    icon: "triangle-exclamation",
                    title: "Couldn't fully close the pane",
                    message: String(err),
                    expiration: Date.now() + 10_000,
                });
                throw err;
            }
        }

        // One confirmation for the whole pane when an agent in it is
        // mid-turn or has tracked processes running (spec §4.6). Then an agent
        // pane is closed IN PLACE (§5.5): returning false stops the layout
        // from removing it now; srv removes it once its agents are down.
        async function beforeNodeDelete(data: TabLayoutData): Promise<boolean> {
            const blockIds = effectiveStack(data).filter(Boolean);
            const busy = await busyMembers(blockIds, paneCloseProbe);
            if (busy.length > 0) {
                const confirmed = await new Promise<boolean>((resolve) =>
                    setPendingCloses((q) => [...q, { busy, resolve }])
                );
                if (!confirmed) return false;
            }
            if (closesWithShutdownLog(blockIds, isAgentBlock)) {
                closeWithShutdownLog(props.tabId, blockIds);
                return false;
            }
            return true;
        }

        return {
            renderContent,
            renderPreview,
            tabId: props.tabId,
            onNodeDelete,
            beforeNodeDelete,
            gapSizePx: tileGapSize(),
        };
    });

    const handleContextMenu = (e: MouseEvent) => {
        const tab = tabData();
        if (!tab || (tab.blockids?.length ?? 0) > 0) return;
        e.preventDefault();
        e.stopPropagation();
        const menu = buildEmptyTabMenu();
        if (menu.length > 0) {
            ContextMenuModel.showContextMenu(menu, e);
        }
    };

    const isEmpty = createMemo(() => (tabData()?.blockids?.length ?? 0) === 0);

    const rootStyle = (): JSX.CSSProperties => ({
        background: "var(--workspace-surface)",
    });

    return (
        <div
            class="flex flex-row flex-grow min-h-0 w-full items-center justify-center overflow-hidden relative"
            style={rootStyle()}
            onContextMenu={handleContextMenu}
        >
            <Show
                when={tabData() != null}
                fallback={<CenteredDiv>Tab Not Found</CenteredDiv>}
            >
                <Show
                    when={isEmpty()}
                    fallback={
                        <ModalLayer scope="tab">
                            <TileLayout
                                contents={tileLayoutContents()}
                                tabAtom={tabAtom()}
                                getCursorPoint={getApi().getCursorPoint}
                            />
                        </ModalLayer>
                    }
                >
                    <EmptyTabIdentity />
                </Show>
            </Show>
            <Show when={pendingClose()}>
                {(pending) => (
                    <ConfirmModal
                        open={true}
                        title="Close this pane?"
                        description="Closing stops these agents. A turn in progress is interrupted, and processes they started are stopped."
                        confirmLabel="Shut down"
                        destructive
                        onConfirm={() => answerPendingClose(true)}
                        onCancel={() => answerPendingClose(false)}
                    >
                        <ul class="list-disc pl-5 text-sm">
                            <For each={pending().busy}>{(m) => <li>{describeBusyMember(m)}</li>}</For>
                        </ul>
                    </ConfirmModal>
                )}
            </Show>
        </div>
    );
}

/**
 * Identity panel shown on an empty tab — logo + the standard "who/what/where"
 * line every desktop app shows during a quiet startup screen: `user@host`,
 * version, git hash. Pulled from cached globals (`getUserName` / `getHostName`)
 * and the host's about-details payload, no IPC on render.
 */
function EmptyTabIdentity(): JSX.Element {
    const details = getApi().getAboutModalDetails();
    const version = details?.version ?? "";
    const gitHash = details?.gitHash ?? "";
    const buildLabel = gitHash ? `${isDev() ? "dev-" : ""}${gitHash}` : "";

    return (
        <div
            class="flex flex-col items-center gap-3 select-none pointer-events-none"
            style={{ opacity: "0.4" }}
        >
            <img
                src={logoUrl}
                alt="AgentMux"
                class="empty-tab-logo"
                style={{ "max-width": "160px", "max-height": "160px" }}
            />
            <div class="flex flex-col items-center gap-1 text-secondary text-[11px] leading-4 text-center">
                <div>{getUserName()}@{getHostName()}</div>
                <Show when={version}>
                    <div>
                        v{version}
                        <Show when={buildLabel}>{" "}({buildLabel})</Show>
                    </div>
                </Show>
            </div>
        </div>
    );
}

export { TabContent };
