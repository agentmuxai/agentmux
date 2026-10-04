// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ConfirmModal } from "@/app/element/confirm-modal";
import type { ModalCloseProps } from "@/app/store/modalmodel";
import type { JSX } from "solid-js";

// Ctrl+Shift+K replaces the focused pane with the launcher, which discards
// what was in it. It used to happen on the key press alone, and the key is
// also delete-line in editors, so it asks first
// (docs/reports/REPORT_KEYBINDINGS_AUDIT_AND_CONSOLIDATION_2026_10_04.md §3.3).
export function ReplacePaneConfirm(props: { onConfirm: () => void } & ModalCloseProps): JSX.Element {
    return (
        <ConfirmModal
            open={true}
            title="Replace this pane with the launcher?"
            description="What's in this pane closes. Pick a new pane type from the launcher."
            confirmLabel="Replace"
            destructive
            onConfirm={() => {
                props.close();
                props.onConfirm();
            }}
            onCancel={props.close}
        />
    );
}
