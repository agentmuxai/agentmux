// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { ContextMenuModel } from "@/app/store/contextmenu";
import { WindowDrag } from "@/element/windowdrag";
import { atoms } from "@/store/global";
import { PLATFORM, PlatformMacOS } from "@/util/platformutil";
import { useAtomValue } from "jotai";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { memo, useCallback, useRef } from "react";
import { createTabBarMenu } from "@/app/menu/base-menus";
import { WindowControls } from "@/app/window/window-controls";
import { SystemStatus } from "@/app/window/system-status";
import "./window-header.scss";

interface WindowHeaderProps {
    workspace: Workspace;
}

const WindowHeader = memo(({ workspace }: WindowHeaderProps) => {
    const windowHeaderRef = useRef<HTMLDivElement>(null);
    const draggerLeftRef = useRef<HTMLDivElement>(null);
    const updateStatusBannerRef = useRef<HTMLButtonElement>(null);
    const configErrorButtonRef = useRef<HTMLElement>(null);

    const settings = useAtomValue(atoms.settingsAtom);
    const fullConfig = useAtomValue(atoms.fullConfigAtom);

    // Handle window drag for Linux (startDragging API works cross-platform)
    const handleHeaderMouseDown = useCallback((e: React.MouseEvent) => {
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
            onMouseDown={handleHeaderMouseDown}
            onContextMenu={handleContextMenu}
        >
            <WindowDrag ref={draggerLeftRef} className="left" />

            <WindowControls
                platform={PLATFORM}
                showNativeControls={PLATFORM === PlatformMacOS && !settings["window:showmenubar"]}
            />

            <SystemStatus
                updateStatusBannerRef={updateStatusBannerRef}
                configErrorRef={configErrorButtonRef}
            />
        </div>
    );
});

export { WindowHeader };
