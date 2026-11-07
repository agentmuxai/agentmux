// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The app/overflow menu, opened from the AgentMux logo. Extracted from TabBar so it can be
// rendered in two places depending on platform:
//   * Windows / Linux — at the LEFT of the tab strip (default position),
//     clear of the right-side caption buttons.
//   * macOS — at the FAR RIGHT of the window header, after the action
//     widgets, so it doesn't crowd the native traffic-light controls on
//     the left. See window-header.tsx.
//
// Every menu action resolves to a module-level store/RPC primitive, so the
// component is fully self-contained (no TabBar state coupling).

import { FlyoutMenu } from "@/app/element/flyoutmenu";
import type { JSX } from "solid-js";
import { closeLogoMenu, createLogoMenu, type LogoMenuProps } from "./logo-menu-state";
import "./logo-menu.scss";

export { closeLogoMenu };

export function LogoMenu(props: LogoMenuProps): JSX.Element {
    const s = createLogoMenu(props);

    return (
        <FlyoutMenu
            anchorClass={s.atRight() ? "logo-menu-anchor logo-menu-anchor--right" : "logo-menu-anchor"}
            items={s.menuItems()}
            placement={s.atRight() ? "bottom-end" : "bottom-start"}
            mirrored={s.atRight()}
            closeRef={s.registerClose}
        >
            <button
                class="logo-menu-btn"
                title="Menu"
                data-drag-region="false"
            >
                {/*
                 * The AgentMux mark (frontend/app/asset/logo-mark.svg, the app icon's four blocks) as inline
                 * rects, not an image or an icon font: they scale evenly with chrome zoom. The mark is the
                 * icon's shape snapped to a 5 x 5 grid of 4 px cells (20 px), so every edge lands on a whole
                 * pixel and the bar, the gaps and the small blocks are all one cell. Colour comes from
                 * `--accent-color` (logo-menu.scss).
                 */}
                <svg width="20" height="20" viewBox="0 0 5 5" fill="currentColor" shape-rendering="crispEdges" aria-hidden="true">
                    <rect x="0" y="0" width="5" height="1" />
                    <rect x="0" y="2" width="2" height="3" />
                    <rect x="3" y="2" width="2" height="1" />
                    <rect x="3" y="4" width="2" height="1" />
                </svg>
            </button>
        </FlyoutMenu>
    );
}
