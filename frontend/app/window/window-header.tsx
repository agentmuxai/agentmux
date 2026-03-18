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

    return (
        <div
            ref={windowHeaderRef}
            class="window-header"
            data-testid="window-header"
            {...dragProps}
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
