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
            {/* Self-contained dark backdrop, theme-independent by design (both
                divs below): this overlay paints its OWN fixed-black scrim +
                chip over whatever pane is underneath, so it never needs to
                react to [data-theme]. */}
            <div
                style={{ transition: "opacity 0.15s ease" }}
                // eslint-disable-next-line no-restricted-syntax -- see comment above
                class="absolute inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm border-2 border-dashed border-accent rounded-lg pointer-events-none"
            >
                <div
                    // eslint-disable-next-line no-restricted-syntax -- see comment above
                    class="text-sm font-medium text-white/90 bg-black/60 px-4 py-2 rounded-md"
                >
                    {message}
                </div>
            </div>
        </Show>
    );
};

export { DragOverlay };
