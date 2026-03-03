// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { getZoomPercentage, zoomFactorAtom, zoomIndicatorVisibleAtom } from "@/app/store/zoom";
import { globalStore } from "@/store/global";
import { useAtomValue } from "jotai";
import "./zoomindicator.scss";

export function ZoomIndicator() {
    const visible = useAtomValue(zoomIndicatorVisibleAtom);
    const zoomPercent = getZoomPercentage(globalStore);

    if (!visible) {
        return null;
    }

    return (
        <div className="zoom-indicator">
            <div className="zoom-indicator-content">{zoomPercent}</div>
        </div>
    );
}
