// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { zoomIndicatorTextAtom, zoomIndicatorVisibleAtom } from "@/app/store/zoom";
import { useAtomValue } from "jotai";
import "./zoomindicator.scss";

export function ZoomIndicator() {
    const visible = useAtomValue(zoomIndicatorVisibleAtom);
    const text = useAtomValue(zoomIndicatorTextAtom);

    if (!visible) {
        return null;
    }

    return (
        <div className="zoom-indicator">
            <div className="zoom-indicator-content">{text}</div>
        </div>
    );
}
