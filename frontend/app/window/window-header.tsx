// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { ContextMenuModel } from "@/app/store/contextmenu";
import { TabBar } from "@/app/tab/tabbar";
import { WindowDrag } from "@/element/windowdrag";
import { useWindowDrag } from "@/app/hook/useWindowDrag";
import { atoms } from "@/store/global";
import { type JSX } from "solid-js";
import { createTabBarMenu } from "@/app/menu/base-menus";
import { SystemStatus } from "@/app/window/system-status";
import { isLinux } from "@/util/platformutil";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./window-header.scss";


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

    // On Windows/macOS: use programmatic startDragging() so that ALL empty
    // space in the header drags the window — including inside child containers
    // (tab-bar, system-status) where data-tauri-drag-region can't reach.
    // On Linux: handled natively via data-tauri-drag-region (GTK drag).
    const handleMouseDown = (e: MouseEvent) => {
        if (e.button !== 0 || isLinux()) return;
        const target = e.target as HTMLElement;
        // Don't drag from interactive elements
        if (target.closest("button, input, select, a, .tab, .action-widget-slot, [data-no-drag]")) return;
        if (e.detail === 2) {
            e.preventDefault();
            getCurrentWindow().toggleMaximize();
        } else {
            e.preventDefault();
            getCurrentWindow().startDragging();
        }
    };

    return (
        <div
            ref={windowHeaderRef}
            class="window-header"
            data-testid="window-header"
            {...dragProps}
            onMouseDown={handleMouseDown}
            onContextMenu={handleContextMenu}
        >
            <WindowDrag ref={draggerLeftRef} class="left" />

            <TabBar workspace={props.workspace} />

            <WindowDrag class="center" />

            <SystemStatus />
        </div>
    );
};

export { WindowHeader };
