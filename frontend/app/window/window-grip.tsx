// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The window's grip: a fixed drag square at the end of the widget bar, before
 * the window controls (Windows, Linux) or the hamburger (macOS). A press on it
 * drags the window, a double-click maximizes it and a right-click opens the
 * title-bar menu, like the rest of the header's empty space; the dots take no
 * events. It sits outside the scrolling tab strip, so it never scrolls or
 * fades with the tabs.
 *
 * docs/specs/SPEC_DRAG_SPACE_AFFORDANCE_2026_10_13.md §5.1, §5.8;
 * docs/reports/REPORT_WINDOW_HEADER_GRIP_PLACEMENT_2026_10_17.md.
 */

import { GripDots } from "@/app/element/GripDots";
import type { JSX } from "solid-js";
import "./window-grip.scss";

export const WindowGrip = (): JSX.Element => (
    <div class="window-grip" data-drag-region="true" data-testid="window-grip" aria-hidden="true">
        <GripDots />
    </div>
);
