// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import React, { forwardRef } from "react";
import { useWindowDrag } from "@/app/hook/useWindowDrag";

import "./windowdrag.scss";

interface WindowDragProps {
    className?: string;
    style?: React.CSSProperties;
    children?: React.ReactNode;
}

const WindowDrag = forwardRef<HTMLDivElement, WindowDragProps>(({ children, className, style }, ref) => {
    const { dragProps } = useWindowDrag();

    return (
        <div
            ref={ref}
            className={clsx(`window-drag`, className)}
            style={style}
            {...dragProps}
        >
            {children}
        </div>
    );
});
WindowDrag.displayName = "WindowDrag";

export { WindowDrag };
