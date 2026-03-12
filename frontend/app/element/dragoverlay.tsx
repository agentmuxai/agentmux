// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { JSX, Show } from "solid-js";

interface DragOverlayProps {
    message: string;
    visible: boolean;
}

const DragOverlay = ({ message, visible }: DragOverlayProps): JSX.Element => {
    return (
        <Show when={visible}>
            <div
                class="absolute inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm border-2 border-dashed border-accent rounded-lg pointer-events-none"
                style={{ transition: "opacity 0.15s ease" }}
            >
                <div class="text-sm font-medium text-white/90 bg-black/60 px-4 py-2 rounded-md">
                    {message}
                </div>
            </div>
        </Show>
    );
};

export { DragOverlay };
