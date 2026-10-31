// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The confirm of shut-down-agent.ts: what ends, and whether its background tasks (dev
// servers) go too. They are kept unless the person ticks the box.

import { createSignal, type JSX } from "solid-js";
import { ConfirmModal } from "@/app/element/confirm-modal";
import type { ModalCloseProps } from "@/app/store/modalmodel";

export function ShutdownConfirm(
    props: { title: string; description: string; onConfirm: (stopBackground: boolean) => void } & ModalCloseProps
): JSX.Element {
    const [stopBackground, setStopBackground] = createSignal(false);
    return (
        <ConfirmModal
            open={true}
            title={props.title}
            description={props.description}
            confirmLabel="Shut down"
            destructive
            onConfirm={() => {
                props.close();
                props.onConfirm(stopBackground());
            }}
            onCancel={props.close}
        >
            <label
                style={{ display: "flex", "align-items": "center", gap: "var(--space-2)", cursor: "pointer", "font-size": "var(--text-base)" }}
            >
                <input
                    type="checkbox"
                    checked={stopBackground()}
                    onChange={(e) => setStopBackground(e.currentTarget.checked)}
                    data-testid="shutdown-stop-background"
                />
                Also stop its background tasks (dev servers)
            </label>
        </ConfirmModal>
    );
}
