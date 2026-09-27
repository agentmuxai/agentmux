// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "Always on top" block meta — kept dependency-free so non-UI code (e.g. the
// split action) can use it. SPEC_FLOATING_PANE_ALWAYS_ON_TOP_2026_09_27 §6.

/** Block meta key for a floating pane's tack, in the `pane:floating_*` family.
 *  Written by the host (`set_floating_always_on_top`), cleared on redock. */
export const FLOATING_ONTOP_META_KEY = "pane:floating_ontop";

/** Is this block's floating pane tacked? */
export function isFloatingOnTop(meta: MetaType | null | undefined): boolean {
    return (meta as Record<string, unknown> | null | undefined)?.[FLOATING_ONTOP_META_KEY] === true;
}
