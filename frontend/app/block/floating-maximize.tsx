// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Maximize / Restore for a torn-off FLOATING pane: the floating half of the
// shared maximize button. Docked panes use `OptMagnifyButton` (layout magnify).
// docs/specs/SPEC_FLOATING_PANE_WINDOW_CONTROLS_REFINEMENTS_2026_10_16.md §3.2, §4.2
//
// The block meta `pane:floating_placement` is what the button shows. The host
// writes it on every toggle; the page writes it when the host reports a change
// it made itself (a drag that restores a maximized floater) or when the window
// resizes into a different placement (the OS acting on its own, on Linux and
// macOS). The click always asks the host to toggle, so a meta that lags never
// makes a click do the wrong thing.

import { getApi } from "@/app/store/app-api";
import * as MOS from "@/app/store/mos";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { IconButton } from "@/element/iconbutton";
import type { JSX } from "solid-js";
import { createMemo, onCleanup, onMount } from "solid-js";

/** How long a burst of resizes must go quiet before the page asks the host whether the window is
 *  maximized: a maximize or restore arrives as several resizes while the window animates. */
export const PLACEMENT_RESIZE_SETTLE_MS = 150;

/** Whether a block's meta says its floater is maximized. Anything but the exact string is normal. */
export function isFloatingMaximized(meta: MetaType | null | undefined): boolean {
    return meta?.["pane:floating_placement"] === "maximized";
}

function blockMeta(blockId: string): MetaType | undefined {
    return MOS.getObjectValue<Block>(MOS.makeORef("block", blockId))?.meta;
}

/** Record the floater's placement in its block's meta, when it differs from what is stored. */
async function storePlacement(blockId: string, maximized: boolean): Promise<void> {
    if (isFloatingMaximized(blockMeta(blockId)) === maximized) return;
    const placement = maximized ? "maximized" : "normal";
    await RpcApi.SetMetaCommand(TabRpcClient, {
        oref: MOS.makeORef("block", blockId),
        meta: { "pane:floating_placement": placement } as MetaType,
    });
}

/**
 * Keep the block's `pane:floating_placement` true to the window. The host tells the page when it
 * changes the placement itself (the button, a drag that restores), and the page asks the host after
 * any resize settles, which catches what the host didn't do: the OS un-maximizing on a compositor
 * move or Super+Down on Linux, and zoom from the keyboard on macOS.
 */
export function useFloatingPlacementSync(label: () => string, blockId: () => string): void {
    onMount(() => {
        let cleaned = false;
        let unlisten: (() => void) | undefined;
        getApi()
            .listen<{ label?: string; placement?: string }>("floating_placement_changed", (ev) => {
                if (ev?.label !== label()) return;
                storePlacement(blockId(), ev.placement === "maximized").catch(console.error);
            })
            .then((u) => (cleaned ? u() : (unlisten = u)))
            .catch(() => {});

        let timer: ReturnType<typeof setTimeout> | undefined;
        const onResize = () => {
            clearTimeout(timer);
            timer = setTimeout(() => {
                getApi()
                    .windows.isMaximized(label())
                    // Unknown (null): keep what is stored rather than guess "normal".
                    .then((maximized) => (maximized == null ? undefined : storePlacement(blockId(), maximized)))
                    .catch(console.error);
            }, PLACEMENT_RESIZE_SETTLE_MS);
        };
        window.addEventListener("resize", onResize);
        onCleanup(() => {
            cleaned = true;
            unlisten?.();
            clearTimeout(timer);
            window.removeEventListener("resize", onResize);
        });
    });
}

/** A floater's Maximize button, which shows Restore (two overlapping windows) while the floater is
 *  maximized. A floater is a real OS window, so its restore is the ordinary window restore and gets
 *  the window glyph, not the tile glyph docked panes use for "back into the tile". */
export function FloatingMaximizeButton(props: { label: string; blockId: string }): JSX.Element {
    useFloatingPlacementSync(
        () => props.label,
        () => props.blockId
    );
    const maximized = createMemo(() =>
        isFloatingMaximized(MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", props.blockId))()?.meta)
    );
    const decl = createMemo<IconButtonDecl>(() => ({
        elemtype: "iconbutton",
        icon: maximized() ? "regular@window-restore" : "regular@window-maximize",
        title: maximized() ? "Restore" : "Maximize",
        click: () => {
            getApi().windows.toggleFloatingMaximize(props.label, props.blockId).catch(console.error);
        },
    }));
    return <IconButton decl={decl()} className="block-frame-magnify" />;
}
