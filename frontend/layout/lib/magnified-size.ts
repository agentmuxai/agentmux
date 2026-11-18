// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How big a magnified pane may be: a share of the layout area it opens over, so 1 fills it and nothing goes
 * past it. A larger number (3 sat in a config and made the pane three times the window, taking Settings off
 * the screen) is read as the largest, and one so small the pane is not magnified as the smallest. The Settings
 * row and the layout read the stored number through here, so a value from an old config is held too.
 */

import { getSettingsKeyAtom } from "@/app/store/block-atom-cache";

export const MAGNIFIED_SIZE_MIN = 0.5;
export const MAGNIFIED_SIZE_MAX = 1;

/** `size` held to [MAGNIFIED_SIZE_MIN, MAGNIFIED_SIZE_MAX]. */
export function clampMagnifiedSize(size: number): number {
    return Math.min(MAGNIFIED_SIZE_MAX, Math.max(MAGNIFIED_SIZE_MIN, size));
}

/** The stored size, held to the range; `undefined` while it is unset or not a number, so each reader's own default applies. */
export function getMagnifiedSizeAtom(): () => number | undefined {
    const stored = getSettingsKeyAtom("window:magnifiedblocksize");
    return () => {
        const size = stored();
        return typeof size === "number" && Number.isFinite(size) ? clampMagnifiedSize(size) : undefined;
    };
}
