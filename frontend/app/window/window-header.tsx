// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { ContextMenuModel } from "@/app/store/contextmenu";
import { TabBar } from "@/app/tab/tabbar";
import { WindowDrag } from "@/element/windowdrag";
import { atoms } from "@/store/global";
import { useAtomValue } from "jotai";
import { memo, useCallback, useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
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

    // Handle header mousedown for window dragging (Linux-compatible)
    const handleHeaderMouseDown = useCallback((e: React.MouseEvent) => {
        if (e.button !== 0) return;
        const target = e.target as HTMLElement;
        // Don't drag if clicking on an interactive element
        if (target.closest("button, input, a, [data-no-drag]")) return;
        e.preventDefault();
        getCurrentWindow().startDragging().catch((err) => {
            console.warn("[WindowHeader] startDragging failed:", err);
        });
    }, []);

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
            onMouseDown={handleHeaderMouseDown}
            onContextMenu={handleContextMenu}
        >
            <WindowDrag ref={draggerLeftRef} className="left" />

            <TabBar workspace={workspace} />

            <SystemStatus />
        </div>
    );
});

export { WindowHeader };
