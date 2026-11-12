// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { zoomIndicatorTargetAtom, zoomIndicatorTextAtom, zoomIndicatorVisibleAtom } from "@/app/store/zoom";
import { createEffect, JSX, on, onMount, Show } from "solid-js";
import { paneBodyBox, zoomIndicatorCenter } from "./zoom-indicator-position";
import "./zoomindicator.scss";

export function ZoomIndicator(): JSX.Element {
    return (
        <Show when={zoomIndicatorVisibleAtom()}>
            <ZoomIndicatorBox />
        </Show>
    );
}

function ZoomIndicatorBox(): JSX.Element {
    let box!: HTMLDivElement;
    // The centre of the pane being zoomed, else the stylesheet's centre of the window. Run as the box mounts (before
    // it is first painted, so it never shows at the window's centre on its way) and again when a zoom names another
    // pane or changes the text, which moves it straight to the new place.
    const place = (): void => {
        const target = zoomIndicatorTargetAtom();
        const centre = zoomIndicatorCenter(
            target == null ? null : paneBodyBox(target),
            { width: box.offsetWidth, height: box.offsetHeight },
            { width: window.innerWidth, height: window.innerHeight }
        );
        if (centre == null) {
            box.style.removeProperty("left");
            box.style.removeProperty("top");
            return;
        }
        box.style.left = `${centre.x}px`;
        box.style.top = `${centre.y}px`;
    };
    onMount(place);
    createEffect(on([zoomIndicatorTargetAtom, zoomIndicatorTextAtom], place, { defer: true }));
    // `data-pane-overlay`: over a browser pane's native window too, a hole is cut in it for the amount.
    return (
        <div ref={box} class="zoom-indicator" data-pane-overlay>
            <div class="zoom-indicator-content">{zoomIndicatorTextAtom()}</div>
        </div>
    );
}
