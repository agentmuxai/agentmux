// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { ContextMenuModel } from "@/app/store/contextmenu";
import { TabBar } from "@/app/tab/tabbar";
import { WindowDrag } from "@/element/windowdrag";
import { atoms } from "@/store/global";
import { useAtomValue } from "jotai";
import { memo, useCallback, useRef } from "react";
import { createTabBarMenu } from "@/app/menu/base-menus";
import { SystemStatus } from "@/app/window/system-status";
import "./window-header.scss";

interface WindowHeaderProps {
    workspace: Workspace;
}

const WindowHeader = memo(({ workspace }: WindowHeaderProps) => {
    const windowHeaderRef = useRef<HTMLDivElement>(null);
    const draggerLeftRef = useRef<HTMLDivElement>(null);

    const fullConfig = useAtomValue(atoms.fullConfigAtom);

    // Handle window header context menu
    const handleContextMenu = useCallback(
        (e: React.MouseEvent) => {
            e.preventDefault();
            const menu = createTabBarMenu(fullConfig);
            ContextMenuModel.showContextMenu(menu.build(), e);
        },
        [fullConfig]
    );

    return (
        <div
            ref={windowHeaderRef}
            className="window-header"
            data-tauri-drag-region
            data-testid="window-header"
            onContextMenu={handleContextMenu}
        >
            <WindowDrag ref={draggerLeftRef} className="left" />

            <TabBar workspace={workspace} />

            <SystemStatus />
        </div>
    );
});

export { WindowHeader };
