// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ContextMenuModel } from "@/app/store/contextmenu";
import { TabBar } from "@/app/tab/tabbar";
import { WindowDrag } from "@/element/windowdrag";
import { useWindowDrag } from "@/app/hook/useWindowDrag.platform";
import { atoms } from "@/store/global";
import { type JSX } from "solid-js";
import { createTabBarMenu } from "@/app/menu/base-menus";
import { SystemStatus } from "@/app/window/system-status";
import { WindowControlsLeft } from "@/app/window/window-controls.platform";
import { HamburgerMenu } from "@/app/window/hamburger-menu";
import { isMacOS } from "@/util/platformutil";
import { Show } from "solid-js";
import "./window-header.platform.scss";


interface WindowHeaderProps {
    workspace: Workspace;
}

const WindowHeader = (props: WindowHeaderProps): JSX.Element => {
    let windowHeaderRef!: HTMLDivElement;
    let draggerLeftRef!: HTMLDivElement;

    const fullConfig = atoms.fullConfigAtom;
    const { dragProps } = useWindowDrag();

    // Handle window header context menu
    const handleContextMenu = (e: MouseEvent) => {
        e.preventDefault();
        const menu = createTabBarMenu(fullConfig());
        ContextMenuModel.showContextMenu(menu.build(), e);
    };

    return (
        <div
            ref={windowHeaderRef}
            class="window-header"
            data-testid="window-header"
            {...dragProps}
            onContextMenu={handleContextMenu}
        >
            <WindowControlsLeft />

            <WindowDrag ref={draggerLeftRef} class="left" />

            <TabBar workspace={props.workspace} />

            <SystemStatus />

            {/* macOS: the hamburger lives at the far right of the header,
                after the action widgets — the native traffic-light controls
                occupy the left, so a left-side hamburger crowds them. On
                Windows/Linux it stays in the tab strip (see TabBar). */}
            <Show when={isMacOS()}>
                <HamburgerMenu position="right" />
            </Show>
        </div>
    );
};

export { WindowHeader };
