// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { zoomIndicatorTextAtom, zoomIndicatorVisibleAtom } from "@/app/store/zoom.platform";
import { JSX, Show } from "solid-js";
import "./zoomindicator.scss";

export function ZoomIndicator(): JSX.Element {
    return (
        <Show when={zoomIndicatorVisibleAtom()}>
            <div class="zoom-indicator">
                <div class="zoom-indicator-content">{zoomIndicatorTextAtom()}</div>
            </div>
        </Show>
    );
}
