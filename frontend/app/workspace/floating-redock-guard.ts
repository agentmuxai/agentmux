// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "A redock is in flight from this window." A floater closes itself the moment its
// tab empties (floating-pane-workspace.tsx), but the backend broadcasts the emptied
// tab BEFORE the RedockFloatingPane call returns, so without this the window would
// be destroyed mid-operation. The drag redock and the Redock button/command both hold
// it; the floater closes when the last holder lets go.
// Each floater is its own renderer, so module state is per floater.

import { createSignal } from "solid-js";

const [holders, setHolders] = createSignal(0);

/** True while any redock from this window is in flight. Reactive. */
export const redockInProgress = (): boolean => holders() > 0;

/** Run `fn` with the guard held; it is released (and the floater may close) when `fn` settles. */
export async function holdRedock<T>(fn: () => Promise<T>): Promise<T> {
    setHolders((n) => n + 1);
    try {
        return await fn();
    } finally {
        setHolders((n) => n - 1);
    }
}
