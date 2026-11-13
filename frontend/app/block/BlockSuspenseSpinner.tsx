// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { BrainSpinner } from "@/app/element/BrainSpinner";
import { createSignal, onCleanup, Show, type JSX } from "solid-js";
import { logWhileShown } from "./pane-cover-log";

/**
 * How long a read in a pane's view may be pending before the pane draws its mark over the view. The view
 * stays on screen while it waits (PaneViewBoundary.tsx), so a read that comes back sooner shows nothing: the
 * mark used to be all the pane showed for that time, and a short read flashed it.
 */
export const PENDING_MARK_AFTER_MS = 500;

/**
 * What a block shows while a read in its view is pending after the cover lifted: nothing at first, its mark
 * once the read has taken `PENDING_MARK_AFTER_MS`. The whole time is written to the `[pane-cover]` log as
 * `suspense-fallback`, from the moment the read is pending.
 */
export function BlockSuspenseSpinner(props: { blockId: string; agent: () => string | undefined }): JSX.Element {
    logWhileShown(props.blockId, "suspense-fallback", props.agent);
    const [late, setLate] = createSignal(false);
    const timer = setTimeout(() => setLate(true), PENDING_MARK_AFTER_MS);
    onCleanup(() => clearTimeout(timer));
    return (
        <Show when={late()}>
            <BrainSpinner />
        </Show>
    );
}
