// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import React, { forwardRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

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
        await getCurrentWindow().startDragging().catch(() => {});
    };

    return (
        <div ref={ref} className={clsx(`window-drag`, className)} style={style} onMouseDown={handleMouseDown}>
            {children}
        </div>
    );
});
WindowDrag.displayName = "WindowDrag";

export { WindowDrag };
