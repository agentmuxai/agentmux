// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The two confirmations of window-close.ts: closing a window while an agent
// in it is working, and Close workspace.

import { createSignal, type JSX } from "solid-js";
import { ConfirmModal } from "@/app/element/confirm-modal";
import type { ModalCloseProps } from "@/app/store/modalmodel";

interface ConfirmText {
    title: string;
    description: string;
}

/** Closing a window with working agents. "Don't ask again" turns the warning off. */
export function WindowCloseConfirm(
    props: ConfirmText & { onConfirm: (dontAskAgain: boolean) => void } & ModalCloseProps
): JSX.Element {
    const [dontAskAgain, setDontAskAgain] = createSignal(false);
    return (
        <ConfirmModal
            open={true}
            title={props.title}
            description={props.description}
            confirmLabel="Close window"
            onConfirm={() => {
                props.close();
                props.onConfirm(dontAskAgain());
            }}
            onCancel={props.close}
        >
            <label style={{ display: "flex", "align-items": "center", gap: "var(--space-2)", cursor: "pointer", "font-size": "var(--text-base)" }}>
                <input
                    type="checkbox"
                    checked={dontAskAgain()}
                    onChange={(e) => setDontAskAgain(e.currentTarget.checked)}
                />
                Don't ask again
            </label>
        </ConfirmModal>
    );
}

/** Close workspace: the one close that deletes. */
export function CloseWorkspaceConfirm(
    props: ConfirmText & { onConfirm: () => Promise<void> } & ModalCloseProps
): JSX.Element {
    return (
        <ConfirmModal
            open={true}
            title={props.title}
            description={props.description}
            confirmLabel="Close workspace"
            destructive
            onConfirm={async () => {
                await props.onConfirm();
                props.close();
            }}
            onCancel={props.close}
        />
    );
}
