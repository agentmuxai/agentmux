// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The two questions asked before a pane of this tab closes: it still has busy agents, or its page would
// ask "Leave site?". Each covers this tab's panes (a tab modal), drawn inside the tab's modal layer.
// docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md row 10.

import { For, Show } from "solid-js";
import type { JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { describeBusyMember, type BusyMember } from "@/app/tab/pane-close-guard";
import { describeLeave, type LeavePrompt } from "@/app/view/browser/leave-prompt";

export function TabCloseConfirms(props: {
    busy: () => { busy: BusyMember[] } | null;
    leave: () => { leaving: LeavePrompt[] } | null;
    answerBusy: (close: boolean) => void;
    answerLeave: (close: boolean) => void;
}): JSX.Element {
    return (
        <>
            <Show when={props.busy()}>
                {(pending) => (
                    <ConfirmModal
                        open={true}
                        scope="tab"
                        title="Close this pane?"
                        description="Closing stops these agents. A turn in progress is interrupted, and processes they started are stopped."
                        confirmLabel="Shut down"
                        destructive
                        onConfirm={() => props.answerBusy(true)}
                        onCancel={() => props.answerBusy(false)}
                    >
                        <ul class="list-disc pl-5 text-sm">
                            <For each={pending().busy}>{(m) => <li>{describeBusyMember(m)}</li>}</For>
                        </ul>
                    </ConfirmModal>
                )}
            </Show>
            <Show when={props.leave()}>
                {(pending) => (
                    <ConfirmModal
                        open={true}
                        scope="tab"
                        title="Close this pane?"
                        description={describeLeave(pending().leaving)}
                        confirmLabel="Close anyway"
                        cancelLabel="Keep open"
                        destructive
                        onConfirm={() => props.answerLeave(true)}
                        onCancel={() => props.answerLeave(false)}
                    />
                )}
            </Show>
        </>
    );
}
