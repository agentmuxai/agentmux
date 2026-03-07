// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import React, { forwardRef } from "react";
import { PLATFORM } from "@/util/platformutil";

import "./windowdrag.scss";

interface WindowDragProps {
    className?: string;
    style?: React.CSSProperties;
    children?: React.ReactNode;
}

// On Linux, window dragging is handled by a native GTK motion-detection handler
// (drag.rs) that distinguishes clicks from drags. Tauri's startDragging() and
// data-tauri-drag-region trigger an immediate Wayland compositor pointer grab
// which swallows button clicks — so we skip them on Linux.
const isLinux = PLATFORM === "linux";

const WindowDrag = forwardRef<HTMLDivElement, WindowDragProps>(({ children, className, style }, ref) => {
    const handleMouseDown = async (e: React.MouseEvent) => {
        if (isLinux) return;
        if (e.button !== 0) return;
        e.preventDefault();
        try {
            const { getCurrentWindow } = await import("@tauri-apps/api/window");
            await getCurrentWindow().startDragging();
        } catch {
            // fallback to CSS -webkit-app-region:drag
        }
    };

    return (
        <div
            ref={ref}
            className={clsx(`window-drag`, className)}
            style={style}
            {...(!isLinux && { "data-tauri-drag-region": true })}
            onMouseDown={handleMouseDown}
        >
            {children}
        </div>
    );
});
WindowDrag.displayName = "WindowDrag";

export { WindowDrag };
