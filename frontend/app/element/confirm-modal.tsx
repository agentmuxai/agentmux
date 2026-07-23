// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Component, JSX, Show, createSignal } from "solid-js";

import { Modal, type ModalScope } from "./modal";
import { ModalBody, ModalFooter, ModalHeader } from "./modal-parts";

// ── ConfirmModal preset ──────────────────────────────────────────────────────
// Common "title + body + Cancel / Confirm" pattern. `destructive` flips
// the confirm button colour to red and routes initial focus to Cancel
// so a stray Enter doesn't delete the thing the user was about to
// double-check. Composes around `Modal` — no new primitive concepts.
//
// The Cancel button carries `data-modal-dismiss` so a rejected backdrop
// click (`closeOnBackdropClick={false}`) nudges it (spec §9).

export interface ConfirmModalProps {
    open: boolean;
    title: string;
    description?: string;
    /** Locks the window by default; pass `tab`/`pane` to scope it. */
    scope?: ModalScope;
    /** Rendered inside the body above the footer buttons. */
    children?: JSX.Element;
    confirmLabel?: string;             // default "OK"
    cancelLabel?: string;              // default "Cancel"
    /** Destructive confirmation — red button + initial focus on Cancel. */
    destructive?: boolean;
    onConfirm: () => void | Promise<void>;
    onCancel: () => void;
}

export const ConfirmModal: Component<ConfirmModalProps> = (props) => {
    const [pending, setPending] = createSignal(false);
    let cancelBtnRef: HTMLButtonElement | undefined;

    const handleConfirm = async () => {
        if (pending()) return;
        try {
            setPending(true);
            await props.onConfirm();
        } finally {
            setPending(false);
        }
    };

    return (
        <Modal
            open={props.open}
            scope={props.scope}
            onClose={() => { if (!pending()) props.onCancel(); }}
            closeOnBackdropClick={!pending()}
            closeOnEscape={!pending()}
            size="sm"
            initialFocus={() => (props.destructive ? (cancelBtnRef ?? null) : null)}
        >
            <ModalHeader title={props.title} description={props.description} />
            <Show when={props.children}>
                <ModalBody>{props.children}</ModalBody>
            </Show>
            <ModalFooter>
                <button
                    ref={cancelBtnRef}
                    type="button"
                    class="modal-btn modal-btn--cancel"
                    data-modal-dismiss
                    onClick={() => { if (!pending()) props.onCancel(); }}
                    disabled={pending()}
                >
                    {props.cancelLabel ?? "Cancel"}
                </button>
                <button
                    type="button"
                    class={`modal-btn modal-btn--confirm${props.destructive ? " modal-btn--destructive" : ""}`}
                    onClick={() => void handleConfirm()}
                    disabled={pending()}
                >
                    {pending() ? "…" : (props.confirmLabel ?? "OK")}
                </button>
            </ModalFooter>
        </Modal>
    );
};
