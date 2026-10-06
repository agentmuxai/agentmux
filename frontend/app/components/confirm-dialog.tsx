// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// ConfirmDialog — modal replacement for window.confirm() with dark-theme styling.
// Used by the file-tree delete flow.

import { onCleanup, onMount, type JSX } from "solid-js";
import { Portal } from "solid-js/web";

import { Button } from "@/app/element/ui";
import "./confirm-dialog.scss";

interface Props {
    title: string;
    message: string;
    confirmLabel?: string;
    onConfirm: () => void;
    onCancel: () => void;
}

export function ConfirmDialog(props: Props): JSX.Element {
    let cancelBtnRef: HTMLButtonElement | undefined;

    onMount(() => cancelBtnRef?.focus());

    const onOverlayKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Escape") { e.preventDefault(); props.onCancel(); }
    };

    const onOverlayPointerDown = (e: PointerEvent) => {
        // Click on the overlay backdrop (not the dialog itself) → cancel.
        if ((e.target as HTMLElement).classList.contains("confirm-overlay")) {
            props.onCancel();
        }
    };

    onCleanup(() => {
        // Nothing to clean up, but having this here makes linters happy.
    });

    return (
        <Portal>
            <div
                class="confirm-overlay"
                onKeyDown={onOverlayKeyDown}
                onPointerDown={onOverlayPointerDown}
            >
                <div class="confirm-dialog" role="dialog" aria-modal="true">
                    <div class="confirm-dialog-title">{props.title}</div>
                    <div class="confirm-dialog-message">{props.message}</div>
                    <div class="confirm-dialog-actions">
                        <Button ref={cancelBtnRef} onClick={props.onCancel}>
                            Cancel
                        </Button>
                        <Button tone="danger" onClick={props.onConfirm}>
                            {props.confirmLabel ?? "Delete"}
                        </Button>
                    </div>
                </div>
            </div>
        </Portal>
    );
}
