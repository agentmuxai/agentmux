// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import type { WardenSection } from "./warden-model";

/** Opens Warden, or focuses the one already in this tab, on `section`. */
export function openWarden(section: WardenSection): Promise<void> {
    const meta = { "warden:section": section };
    return openOrFocusPaneByView("warden", { meta: { view: "warden", ...meta } }, meta);
}
