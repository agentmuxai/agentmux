// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { QuickTips } from "@/app/element/quicktips";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { showZoomIndicator } from "@/app/store/zoom.platform";
import { WOS } from "@/store/global";
import { fireAndForget } from "@/util/util";
import { createSignal, onMount, type JSX } from "solid-js";

const MIN_ZOOM = 0.5;
const MAX_ZOOM = 2.0;
const KEYBOARD_STEP = 0.1;
const WHEEL_STEP = 0.05;

/**
 * HelpViewModel - shows QuickTips content with Ctrl+/- / Ctrl+Wheel zoom.
 * Zoom is persisted in block meta as "help:zoom".
 */
class HelpViewModel implements ViewModel {
    viewType: string;
    blockId: string;

    constructor(blockId: string) {
        this.viewType = "help";
        this.blockId = blockId;
    }

    get viewComponent(): ViewComponent {
        return HelpView as unknown as ViewComponent;
    }
}

function HelpView({ model }: { model: HelpViewModel }): JSX.Element {
    const [zoom, setZoom] = createSignal(1.0);

    onMount(() => {
        const blockData = WOS.getWaveObjectAtom<Block>(`block:${model.blockId}`)();
        const saved = blockData?.meta?.["help:zoom"];
        if (typeof saved === "number" && saved >= MIN_ZOOM && saved <= MAX_ZOOM) {
            setZoom(saved);
        }
    });

    const adjustZoom = (delta: number) => {
        const next = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round((zoom() + delta) * 100) / 100));
        setZoom(next);
        fireAndForget(() =>
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", model.blockId),
                meta: { "help:zoom": Math.abs(next - 1.0) < 0.01 ? null : next },
            })
        );
        showZoomIndicator(`${Math.round(next * 100)}%`);
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (!e.ctrlKey && !e.metaKey) return;
        if (e.key === "=" || e.key === "+") { e.preventDefault(); e.stopPropagation(); adjustZoom(KEYBOARD_STEP); }
        else if (e.key === "-")            { e.preventDefault(); e.stopPropagation(); adjustZoom(-KEYBOARD_STEP); }
        else if (e.key === "0")            { e.preventDefault(); e.stopPropagation(); setZoom(1.0); showZoomIndicator("100%"); }
    };

    const handleWheel = (e: WheelEvent) => {
        if (!e.ctrlKey && !e.metaKey) return;
        e.preventDefault();
        e.stopPropagation();
        adjustZoom(e.deltaY > 0 ? -WHEEL_STEP : WHEEL_STEP);
    };

    return (
        <div
            class="overflow-auto w-full h-full outline-none"
            tabIndex={0}
            onKeyDown={handleKeyDown}
            onWheel={handleWheel}
        >
            <div style={{ zoom: zoom(), padding: "10px 5px" }}>
                <QuickTips />
            </div>
        </div>
    );
}

export { HelpViewModel };
