// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "Always on top" for floating panes — the header tack.
// docs/specs/SPEC_FLOATING_PANE_ALWAYS_ON_TOP_2026_09_27.md
//
// While tacked, a floating pane stays above every window of this AgentMux
// instance (not above other apps). The host applies the z-order and records
// `pane:floating_ontop` on the block; the button reads that meta, so the
// block is the single source of truth, and a reloaded floater re-applies it.

import { hostHas } from "@/app/host/host-caps";
import { getApi } from "@/app/store/app-api";
import * as MOS from "@/app/store/mos";
import { ToggleIconButton } from "@/element/iconbutton";
import { isWindows } from "@/util/platformutil";
import type { SignalAtom } from "@/util/util";
import type { JSX } from "solid-js";
import { createEffect, createMemo } from "solid-js";

import { isFloatingOnTop } from "./floating-ontop-meta";

export { FLOATING_ONTOP_META_KEY, isFloatingOnTop } from "./floating-ontop-meta";

/** Show the tack? Floaters only, on a host that supports it. Phase 1 is
 *  Windows-only (spec §7) — the host command errors elsewhere. */
export function canTackFloatingPane(floatingLabel: string | null): boolean {
    return floatingLabel != null && hostHas("floatingAlwaysOnTop") && isWindows();
}

/** Tack / untack a floating pane. Errors are logged, not thrown: a click that
 *  fails leaves the meta — and so the button — unchanged. */
export function setFloatingOnTop(label: string, blockId: string, on: boolean): void {
    getApi().windows.setFloatingAlwaysOnTop(label, blockId, on).catch(console.error);
}

/** Re-apply a stored tack when a floating window (re)loads (spec §6.2): the
 *  window is a fresh HWND after a page reload or a pool promote, so the host
 *  has to be told again. Runs once per block, as soon as the block has
 *  loaded. Call inside the floating window's component. */
export function useReapplyFloatingOnTop(label: () => string, blockId: () => string | undefined): void {
    let checkedBlock: string | null = null;
    createEffect(() => {
        const id = blockId();
        const lbl = label();
        if (!id || id === checkedBlock) return;
        const block = MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", id))();
        if (!block) return; // not loaded yet — the effect re-runs when it is
        checkedBlock = id;
        if (isFloatingOnTop(block.meta) && canTackFloatingPane(lbl.startsWith("floating-") ? lbl : null)) {
            setFloatingOnTop(lbl, id, true);
        }
    });
}

/** Header tack for a floating pane: a `toggleiconbutton` like the agent pane's
 *  Stash, highlighted in the theme accent color while on (iconbutton.scss
 *  `.toggle.active`), placed with the window controls before Maximize. */
export function FloatingAlwaysOnTopButton(props: { label: string; blockId: string }): JSX.Element {
    const onTop = createMemo(() =>
        isFloatingOnTop(MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", props.blockId))()?.meta)
    );
    // A fake SignalAtom, as Stash does: the read tracks the block meta; the
    // write asks the host, whose meta write-through flips the read.
    const active = (() => onTop()) as SignalAtom<boolean>;
    active._set = (next: boolean | ((prev: boolean) => boolean)) => {
        const on = typeof next === "function" ? next(onTop()) : next;
        setFloatingOnTop(props.label, props.blockId, on);
    };
    const decl: ToggleIconButtonDecl = {
        elemtype: "toggleiconbutton",
        icon: "thumbtack",
        title: "Always on top",
        active,
    };
    return <ToggleIconButton decl={decl} className="block-frame-ontop" />;
}
