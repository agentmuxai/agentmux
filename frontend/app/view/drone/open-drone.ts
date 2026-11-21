// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { openOrFocusPaneByView } from "@/app/store/block-component-registry";

/** The pane meta that asks a Drone pane to open a saved drone; the pane clears it once the graph is shown. */
export const DRONE_OPEN_META_KEY = "drone:open";

/** The drone id in a pane's meta, or null when none is asked for. */
export function droneOpenRequest(meta: Record<string, unknown> | null | undefined): string | null {
    const id = meta?.[DRONE_OPEN_META_KEY];
    return typeof id === "string" && id ? id : null;
}

/** Opens the Drone pane (this tab's, or a new one) with this drone's graph on the canvas. */
export function openDroneGraph(droneId: string): Promise<void> {
    const meta = { [DRONE_OPEN_META_KEY]: droneId };
    return openOrFocusPaneByView("drone", { meta: { view: "drone", ...meta } }, meta);
}
