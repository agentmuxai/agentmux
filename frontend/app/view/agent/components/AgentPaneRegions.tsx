// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Three regions of the agent pane's layout: the transcript, the view-mode popover and the composer with its slash panels. */

import { Show, type Accessor, type ComponentProps, type JSX } from "solid-js";
import type { AgentViewModel } from "../agent-model";
import { viewChipLabel, type createAgentViewControls } from "../agent-view-controls";
import type { useAgentCommands } from "../hooks/useAgentCommands";
import type { useOfflineInput } from "../offline/useOfflineInput";
import { AgentDocumentView } from "./AgentDocumentView";
import { AgentFooter } from "./AgentFooter";
import { AgentViewPopover } from "./AgentViewPopover";
import { HistoryLoadingRow } from "./HistoryLoadingRow";
import { NewActivityPill } from "./NewActivityPill";
import { SlashCommandPicker } from "./SlashCommandPicker";
import { SlashHelpPanel } from "./SlashHelpPanel";

type DocProps = ComponentProps<typeof AgentDocumentView>;

/** The scroll region: `.agent-document` inside AgentDocumentView is absolutely positioned to fill it. The working row is a sibling below the dock, not an overlay on this box (SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01.md). */
export function AgentTranscriptRegion(
    props: Pick<
        DocProps,
        | "documentNodes"
        | "documentStateAtom"
        | "onOpenHistory"
        | "onAgentErrorLogin"
        | "onLoadOlder"
        | "loadingOlder"
        | "hasOlderHistory"
        | "followingRef"
        | "newActivityRef"
        | "scrollCommand"
        | "scrollToBottomRef"
        | "highlightNodeId"
        | "registerHistoryReadyCallback"
        | "zoomFactor"
        | "blockId"
        | "layoutView"
    > & {
        readiness: ComponentProps<typeof HistoryLoadingRow>["readiness"];
        newActivityVisible: Accessor<boolean>;
        onJumpToNewActivity: () => void;
    },
): JSX.Element {
    return (
        <div class="agent-document-scroll-region">
            <HistoryLoadingRow readiness={props.readiness} />
            <AgentDocumentView
                documentNodes={props.documentNodes}
                documentStateAtom={props.documentStateAtom}
                onOpenHistory={props.onOpenHistory}
                onAgentErrorLogin={props.onAgentErrorLogin}
                onLoadOlder={props.onLoadOlder}
                loadingOlder={props.loadingOlder}
                // The one paging gate: the list pages only while this is true.
                hasOlderHistory={props.hasOlderHistory}
                followingRef={props.followingRef}
                newActivityRef={props.newActivityRef}
                scrollCommand={props.scrollCommand}
                scrollToBottomRef={props.scrollToBottomRef}
                highlightNodeId={props.highlightNodeId}
                registerHistoryReadyCallback={props.registerHistoryReadyCallback}
                zoomFactor={props.zoomFactor}
                blockId={props.blockId}
                layoutView={props.layoutView}
            />
            <NewActivityPill visible={props.newActivityVisible} onJump={props.onJumpToNewActivity} />
        </div>
    );
}

/** Hangs from the chip, or from the strip while there is none (opened from the menu or the palette). Keyed by which: the panel reads its anchor once, and a change of view can remove the chip under it. */
export function AgentViewPopoverHost(props: { controls: ReturnType<typeof createAgentViewControls>; rootEl: () => HTMLElement | undefined }): JSX.Element {
    return (
        <Show when={props.controls.popoverOpen() ? (viewChipLabel(props.controls.resolved()) ? "chip" : "strip") : undefined} keyed>
            {(anchor) => (
                <AgentViewPopover
                    controls={props.controls}
                    anchor={props.rootEl()?.querySelector(anchor === "chip" ? "[data-agent-view-chip]" : ".agent-composer-strip") ?? undefined}
                />
            )}
        </Show>
    );
}

/** The composer, with the slash help panel and command picker above it. */
export function AgentComposerRegion(props: {
    commands: ReturnType<typeof useAgentCommands>;
    offline: ReturnType<typeof useOfflineInput>;
    model: AgentViewModel;
    agentName: Accessor<string>;
    onTyping: () => void;
    isComposerEmptyRef: (fn: () => boolean) => void;
}): JSX.Element {
    return (
        <div class="agent-composer-region" data-toast-avoid>
            <Show when={props.commands.helpVisible()}>
                <SlashHelpPanel
                    commands={props.commands.availableCommands()}
                    onInvoke={(cmd) => {
                        props.commands.closeHelp();
                        // Through sendWithState, not sendMessage: a bare sendMessage assumes no turn is running, so
                        // /login from this panel mid-turn would restart, and kill, that turn.
                        void props.commands.sendWithState(`/${cmd.name}`);
                    }}
                    onClose={props.commands.closeHelp}
                />
            </Show>
            <Show when={props.commands.pickerSpec()}>
                {(spec) => (
                    <SlashCommandPicker
                        spec={spec()}
                        onSelect={props.commands.resolvePicker}
                        onDismiss={props.commands.dismissPicker}
                    />
                )}
            </Show>
            <AgentFooter
                agentName={props.agentName()}
                onSendMessage={props.offline.submit}
                onTyping={props.onTyping}
                onStopAgent={props.commands.escapeOnEmptyComposer}
                onRecallLatestQueued={props.commands.recallLatestHeld}
                getCompletions={props.commands.completions}
                viewModel={props.model}
                isComposerEmptyRef={props.isComposerEmptyRef}
            />
        </div>
    );
}
