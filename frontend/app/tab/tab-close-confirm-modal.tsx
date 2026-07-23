// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";
import type { JSX } from "solid-js";
import { ConfirmModal } from "@/element/modal";
import { getObjectValue, makeORef } from "../store/wos";

export function TabCloseConfirmModal(props: {
    tabId: string;
    onConfirm: (skipFuture: boolean) => void;
    onCancel: () => void;
}): JSX.Element {
    const [skipFuture, setSkipFuture] = createSignal(false);
    const tabName = () => getObjectValue<Tab>(makeORef("tab", props.tabId))?.name ?? "this tab";

    return (
        <ConfirmModal
            open={true}
            scope="window"
            title={`Close "${tabName()}"?`}
            description="This tab and all its panes will be closed."
            confirmLabel="Close tab"
            destructive={true}
            onConfirm={() => props.onConfirm(skipFuture())}
            onCancel={props.onCancel}
        >
            <label style={{ display: "flex", "align-items": "center", gap: "8px", cursor: "pointer", "font-size": "13px" }}>
                <input
                    type="checkbox"
                    checked={skipFuture()}
                    onChange={(e) => setSkipFuture(e.currentTarget.checked)}
                />
                Don't ask again
            </label>
        </ConfirmModal>
    );
}
