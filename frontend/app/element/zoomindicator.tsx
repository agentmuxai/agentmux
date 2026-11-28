// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { t } from "@/app/i18n/i18n";
import {
    zoomIndicatorLabelAtom,
    zoomIndicatorTargetAtom,
    zoomIndicatorTextAtom,
    zoomIndicatorVisibleAtom,
} from "@/app/store/zoom";
import { splitSubject } from "@/app/store/zoom-parts";
import { createEffect, JSX, on, onCleanup, onMount, Show } from "solid-js";
import { paneBodyBox, zoomIndicatorCenter, zoomTargetBox } from "./zoom-indicator-position";
import "./zoomindicator.scss";

/** The amount with the name of the part it belongs to, when the pane has parts on screen: "File tree 130%". */
const indicatorText = (): string => {
    const label = zoomIndicatorLabelAtom();
    return label ? `${t(label)} ${zoomIndicatorTextAtom()}` : zoomIndicatorTextAtom();
};

export function ZoomIndicator(): JSX.Element {
    return (
        <>
            {/* A live region that is always there, so a screen reader announces each amount as it changes. */}
            <div class="zoom-indicator-live" role="status" aria-live="polite">
                {zoomIndicatorVisibleAtom() ? indicatorText() : ""}
            </div>
            <Show when={zoomIndicatorVisibleAtom()}>
                <ZoomIndicatorBox />
            </Show>
        </>
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
            target == null ? null : zoomTargetBox(target),
            { width: box.offsetWidth, height: box.offsetHeight },
            { width: window.innerWidth, height: window.innerHeight },
            target == null ? null : paneBodyBox(splitSubject(target).blockId)
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
    // Later changes (each wheel tick of a zoom) are placed once, at the next frame, not inside the tick: the zoom just
    // changed the pane's size, so measuring it there made the browser lay the page out in the middle of the wheel
    // listener (28 to 37 ms a tick in the installed build's log), once per tick; a frame has one place to measure.
    let frame = 0;
    const placeAtNextFrame = (): void => {
        if (frame) return;
        frame = requestAnimationFrame(() => {
            frame = 0;
            place();
        });
    };
    onCleanup(() => {
        if (frame) cancelAnimationFrame(frame);
    });
    createEffect(on([zoomIndicatorTargetAtom, zoomIndicatorTextAtom, zoomIndicatorLabelAtom], placeAtNextFrame, { defer: true }));
    // `data-pane-overlay`: over a browser pane's native window too, a hole is cut in it for the amount.
    return (
        <div ref={box} class="zoom-indicator" data-pane-overlay>
            <div class="zoom-indicator-content">{indicatorText()}</div>
        </div>
    );
}
