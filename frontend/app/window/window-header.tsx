// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { ContextMenuModel } from "@/app/store/contextmenu";
import { TabBar } from "@/app/tab/tabbar";
import { WindowDrag } from "@/element/windowdrag";
import { atoms } from "@/store/global";
import { PLATFORM } from "@/util/platformutil";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useAtomValue } from "jotai";
import { memo, useCallback, useRef } from "react";
import { createTabBarMenu } from "@/app/menu/base-menus";
import { SystemStatus } from "@/app/window/system-status";
import "./window-header.scss";

const isLinux = PLATFORM === "linux";

interface WindowHeaderProps {
    workspace: Workspace;
}

const WindowHeader = memo(({ workspace }: WindowHeaderProps) => {
    const windowHeaderRef = useRef<HTMLDivElement>(null);
    const draggerLeftRef = useRef<HTMLDivElement>(null);

    const fullConfig = useAtomValue(atoms.fullConfigAtom);

    // On non-Linux, use Tauri's startDragging() for window dragging.
    // On Linux, drag.rs handles this via GTK motion detection to avoid
    // Wayland compositor pointer grab that kills header button clicks.
    const handleHeaderMouseDown = useCallback((e: React.MouseEvent) => {
        if (isLinux) return;
        if (e.button !== 0) return;
        const target = e.target as HTMLElement;
        if (target.closest("button, input, a, [data-no-drag]")) return;
        e.preventDefault();
        getCurrentWindow().startDragging().catch(() => {});
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
            data-testid="window-header"
            {...(!isLinux && { "data-tauri-drag-region": true })}
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
