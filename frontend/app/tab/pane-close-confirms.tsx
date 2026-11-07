// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The two questions asked before a PANE closes: it still has busy agents, or its page would ask "Leave site?".
// They are about that pane alone, so each is a pane modal: it covers the pane being closed and nothing else,
// and every other pane, the tab bar and the window stay usable while it waits. (Closing a TAB is the one close
// that stays tab-wide: `tab-close-confirm-modal.tsx`.)
// docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md row 10.

import { For } from "solid-js";
import type { JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { openPaneModal, paneRequests, type PaneRequestProps } from "@/app/store/pane-requests";
import { describeBusyMember, type BusyMember } from "@/app/tab/pane-close-guard";
import { describeLeave, type LeavePrompt } from "@/app/view/browser/leave-prompt";
import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";

type Answer = { onAnswer: (close: boolean) => void } & PaneRequestProps;

export function CloseBusyConfirm(props: { busy: BusyMember[] } & Answer): JSX.Element {
    const answer = (close: boolean) => {
        props.close();
        props.onAnswer(close);
    };
    return (
        <ConfirmModal
            open={true}
            scope="pane"
            title="Close this pane?"
            description="Closing stops these agents. A turn in progress is interrupted, and processes they started are stopped."
            confirmLabel="Shut down"
            destructive
            onConfirm={() => answer(true)}
            onCancel={() => answer(false)}
        >
            <ul class="list-disc pl-5 text-sm">
                <For each={props.busy}>{(m) => <li>{describeBusyMember(m)}</li>}</For>
            </ul>
        </ConfirmModal>
    );
}

export function LeaveSiteConfirm(props: { leaving: LeavePrompt[] } & Answer): JSX.Element {
    const answer = (close: boolean) => {
        props.close();
        props.onAnswer(close);
    };
    return (
        <ConfirmModal
            open={true}
            scope="pane"
            title="Close this pane?"
            description={describeLeave(props.leaving)}
            confirmLabel="Close anyway"
            cancelLabel="Keep open"
            destructive
            onConfirm={() => answer(true)}
            onCancel={() => answer(false)}
        />
    );
}

/** The block the pane that holds `blockId` shows: a pane draws (and hosts the modals of) its active tab only. */
function shownBlockOf(tabId: string, blockId: string): string {
    const data = getLayoutModelForTabById(tabId)?.getNodeByBlockId(blockId)?.data;
    return data?.activeBlockId ?? data?.blockId ?? blockId;
}

/** Questions already open, by pane and kind: a second close of the same pane shares the first's answer. */
const asking = new Map<string, Promise<boolean>>();

/**
 * Asks `Component` in the pane that holds `blockId` (the one on screen, when it is a stack) and resolves with
 * the person's answer: true closes the pane. The pane going away first answers no.
 */
export function askInPane<P extends object>(
    tabId: string,
    blockId: string,
    source: "close-busy" | "close-leave",
    Component: (props: P & Answer) => JSX.Element,
    props: P
): Promise<boolean> {
    const host = shownBlockOf(tabId, blockId);
    const key = `${host}:${source}`;
    const open = asking.get(key);
    if (open) return open;
    const answered = new Promise<boolean>((resolve) => {
        openPaneModal(
            host,
            Component as (props: Record<string, unknown> & PaneRequestProps) => JSX.Element,
            { ...props, onAnswer: resolve },
            { source, onGone: () => resolve(false), text: "Closing this pane needs your answer", what: "asks before closing this pane" }
        );
    }).finally(() => asking.delete(key));
    asking.set(key, answered);
    return answered;
}

/** Tests only: forget every open question and its request. */
export function __resetPaneCloseConfirms(): void {
    for (const key of asking.keys()) {
        const host = key.slice(0, key.lastIndexOf(":"));
        for (const r of paneRequests.of(host)) paneRequests.close(r.id);
    }
    asking.clear();
}
