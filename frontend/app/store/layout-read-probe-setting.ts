// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getSettingsKeyAtom } from "@/store/global";
import { createEffect, createRoot } from "solid-js";
import { layoutReadProbe, wrapLayoutReads } from "./layout-read-probe";

/**
 * Turns the layout-read probe (`layout-read-probe.ts`) on while `debug:layoutreadprobe` is true. Off, nothing on the page
 * is wrapped, so a page that never sets it runs the browser's own calls.
 */
export function installLayoutReadProbe(): void {
    let unwrap: (() => void) | null = null;
    createRoot(() =>
        createEffect(() => {
            const on = getSettingsKeyAtom("debug:layoutreadprobe")() === true;
            layoutReadProbe.setEnabled(on);
            if (on && unwrap == null) unwrap = wrapLayoutReads(layoutReadProbe);
            if (!on && unwrap != null) {
                unwrap();
                unwrap = null;
            }
        })
    );
}
