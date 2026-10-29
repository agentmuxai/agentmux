// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Block, resolveEffectiveViewType } from "@/app/block/block";
import { PaneLeafChrome } from "@/app/tab/pane-leaf-chrome";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { ModalLayer } from "@/element/ModalLayer";
import { CenteredDiv } from "@/element/quickelems";
import logoUrl from "@/app/asset/logo-brain.svg?url";
import { ContentRenderer, NodeModel, PreviewRenderer, TileLayout } from "@/layout/index";
import type { BeforeDeleteHooks } from "@/layout/lib/types";
import { effectiveStack } from "@/layout/lib/layoutStack";
import { TileLayoutContents } from "@/layout/lib/types";
import { atoms, createBlock, getApi, getHostName, getUserName, isDev, pushNotification } from "@/store/global";
import * as services from "@/store/services";
import * as MOS from "@/store/mos";
import { buildPaneWidgetMenuItems } from "@/app/window/action-widgets-config";
import { ConfirmModal } from "@/app/element/confirm-modal";
import {
    busyMembers,
    closesWithShutdownLog,
    describeBusyMember,
    membersToProbe,
    type BusyMember,
    type PaneCloseProbe,
} from "@/app/tab/pane-close-guard";
import { closeWithShutdownLog, isClosingWithLog } from "@/app/view/agent/shutdown/close-with-log";
import { agentsKeepRunning, noteKeepsRunning, parkedAgentOf } from "@/app/view/agent/shutdown/keeps-running";
import {
    describeLeave,
    leavePrompts,
    notePersonClose,
    siteOf,
    type LeavePrompt,
    type LeaveProbe,
} from "@/app/view/browser/leave-prompt";
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

/** The quiet notice for a pane that stays open because the person answered No. */
function notKept(why: string): void {
    pushNotification({
        icon: "fa-triangle-exclamation",
        title: "Not closed",
        message: why,
        timestamp: new Date().toISOString(),
        type: "info",
        expiration: Date.now() + 5000,
    });
}

/** Through the view aliases (legacy "forge" is an agent pane too), as
 *  block.tsx requires — not a raw `meta.view === "agent"` check. */
function isAgentBlock(blockId: string): boolean {
    const view = MOS.getObjectValue<MuxObj>(MOS.makeORef("block", blockId))?.meta?.["view"];
    return resolveEffectiveViewType(typeof view === "string" ? view : "") === "agent";
}

function isBrowserBlock(blockId: string): boolean {
    const view = MOS.getObjectValue<MuxObj>(MOS.makeORef("block", blockId))?.meta?.["view"];
    return resolveEffectiveViewType(typeof view === "string" ? view : "") === "browser";
}

/** Whether a browser pane's page would ask "Leave site?" (`leave-prompt.ts`). */
const leaveProbe: LeaveProbe = {
    isBrowser: isBrowserBlock,
    asks: async (blockId) => (await getApi().browserPanes.hibernationCheck?.(blockId))?.leave_prompt === true,
    site: (blockId) =>
        siteOf(MOS.getObjectValue<MuxObj>(MOS.makeORef("block", blockId))?.meta?.["url"] as string | undefined),
};

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
        for (const pending of pendingLeaves()) pending.resolve(false);
    });

    // The same for a browser page that would ask "Leave site?" when its pane
    // is closed by the person: AgentMux's own dialog, naming the site.
    const [pendingLeaves, setPendingLeaves] = createSignal<
        { leaving: LeavePrompt[]; resolve: (close: boolean) => void }[]
    >([]);
    const pendingLeave = () => pendingLeaves()[0] ?? null;
    const answerPendingLeave = (close: boolean) => {
        const [head, ...rest] = pendingLeaves();
        setPendingLeaves(rest);
        head?.resolve(close);
    };

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
        // While agents keep running (the default) a close only takes the pane away: srv parks the
        // agent, so there is no busy prompt and no shutdown log, just a note that it runs on.
        //
        // Only what can be answered is asked: an agent for a turn in progress, a browser for "Leave
        // site?". Every other pane has nothing to ask, so its close makes no srv call and the pane
        // leaves in the same frame (SPEC_PANE_CLOSING_INDICATOR_AND_FASTER_CLOSE_2026_10_28.md §3).
        // `hooks.asking` marks a person's answer being awaited, when the prompt is the indicator.
        async function beforeNodeDelete(data: TabLayoutData, hooks?: BeforeDeleteHooks): Promise<boolean> {
            const blockIds = effectiveStack(data).filter(Boolean);
            // Already closing (an agent winding down under its shutdown log): a second click
            // neither probes srv again nor asks the prompts again.
            if (isClosingWithLog(blockIds)) return false;
            const parked = new Map(
                agentsKeepRunning() ? blockIds.flatMap((id) => { const a = parkedAgentOf(id); return a ? [[id, a] as const] : []; }) : []
            );
            const busy = await busyMembers(membersToProbe(blockIds, isAgentBlock, (id) => parked.has(id)), paneCloseProbe);
            if (busy.length > 0) {
                const answer = new Promise<boolean>((resolve) => setPendingCloses((q) => [...q, { busy, resolve }]));
                if (!(await (hooks ? hooks.asking(answer) : answer))) {
                    notKept(busy.map(describeBusyMember).join("; "));
                    return false;
                }
            }
            const leaving = await leavePrompts(blockIds, leaveProbe);
            if (leaving.length > 0) {
                const answer = new Promise<boolean>((resolve) => setPendingLeaves((q) => [...q, { leaving, resolve }]));
                if (!(await (hooks ? hooks.asking(answer) : answer))) {
                    notKept(describeLeave(leaving));
                    return false;
                }
            }
            // The agents srv parks close at once; any other agent block still winds down under its log.
            const winds = (id: string) => isAgentBlock(id) && !parked.has(id);
            if (closesWithShutdownLog(blockIds, winds)) {
                void closeWithShutdownLog(props.tabId, blockIds, winds);
                return false;
            }
            noteKeepsRunning([...parked.values()]);
            // The host's log says the person closed these panes.
            notePersonClose(blockIds.filter(isBrowserBlock));
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
        // Live, like the pane "+" picker: follows widgets hidden or shown while it's open.
        ContextMenuModel.showLiveContextMenu(buildEmptyTabMenu, e);
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
            <Show when={pendingLeave()}>
                {(pending) => (
                    <ConfirmModal
                        open={true}
                        title="Close this pane?"
                        description={describeLeave(pending().leaving)}
                        confirmLabel="Close anyway"
                        cancelLabel="Keep open"
                        destructive
                        onConfirm={() => answerPendingLeave(true)}
                        onCancel={() => answerPendingLeave(false)}
                    />
                )}
            </Show>
        </div>
    );
}

/**
 * Identity panel shown on an empty tab — logo + the standard "who/what/where"
 * line every desktop app shows during a quiet startup screen: `user@host`,
 * version, git hash. `user@host` is srv's (`getUserName` / `getHostName`, from
 * its `srvinfo`); the rest is the host's about-details payload. No IPC on render.
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
