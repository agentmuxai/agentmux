// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { JSX } from "solid-js";
import type { MenuPositionResult } from "./menu-position";

/**
 * The style that holds a panel placed above its anchor by its bottom edge. A panel pinned by its top edge grows
 * downward when its content grows (a tooltip's rows arriving after it opens), over the anchor and the pointer
 * on it, until the next placement pass moves it; pinned by the bottom edge it grows upward, away from the anchor.
 * A panel on any other side keeps the style it was given.
 */
export function holdBottomEdge(
    result: Pick<MenuPositionResult, "style" | "placement">,
    floatingHeight: number,
    viewportHeight: number,
): JSX.CSSProperties {
    if (!result.placement?.startsWith("top")) return result.style;
    const top = Number.parseFloat(String(result.style.top));
    if (!Number.isFinite(top)) return result.style;
    return {
        position: "fixed",
        left: result.style.left,
        bottom: `${Math.round(viewportHeight - (top + floatingHeight))}px`,
    };
}
