// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import React, { forwardRef } from "react";

import "./windowdrag.scss";

interface WindowDragProps {
    className?: string;
    style?: React.CSSProperties;
    children?: React.ReactNode;
}

const WindowDrag = forwardRef<HTMLDivElement, WindowDragProps>(({ children, className, style }, ref) => {
    const handleMouseDown = async (e: React.MouseEvent) => {
        if (e.button !== 0) return;
        e.preventDefault();
        try {
            const { getCurrentWindow } = await import("@tauri-apps/api/window");
            await getCurrentWindow().startDragging();
<<<<<<< Updated upstream
        } catch {
            // fallback to CSS -webkit-app-region:drag
        }
    };

    return (
        <div
            ref={ref}
            className={clsx(`window-drag`, className)}
            style={style}
            data-tauri-drag-region
            onMouseDown={handleMouseDown}
        >
>>>>>>> Stashed changes
            {children}
        </div>
    );
});
WindowDrag.displayName = "WindowDrag";

export { WindowDrag };
