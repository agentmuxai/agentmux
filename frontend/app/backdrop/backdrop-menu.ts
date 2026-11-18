// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pane's Backdrop menu: a submenu of the pane's header menu laid out like Pane Color (None, the 12 hues, then
 * Black and White, each an exact-colour square), and the same row behaviour: a pick applies at once and the menu stays
 * open, so backdrops can be tried one after another. A pane that already has a custom colour or a fixed polarity keeps
 * it (the menu does not offer to make one; None clears the record), and a pick keeps the polarity the pane has.
 */

import { setBlockMeta } from "@/app/store/block-meta";
import { MOS } from "@/app/store/global";
import { MENU_ICON } from "@/app/util/menu-icons";
import { openMenuAt } from "@/app/element/ui/open-menu-at";
import { slotById } from "@/app/theme/palette";
import { isLightThemeActive } from "../block/pane-identity";
import { parseBackdrop } from "./meta";
import { builtinOptions, builtinRecord, schemeFor } from "./picker-model";

type LiveBlock = () => Block | undefined;

/** The backdrop rows: None, the 12 hues, a divider, Black and White; the checked row follows the pane while the menu is open. */
export function backdropMenuItems(blockData: Block, live?: LiveBlock): ContextMenuItem[] {
    const now = () => live?.() ?? blockData;
    const record = (b: Block) => parseBackdrop(b?.meta?.backdrop);
    const pickedId = (b: Block) => (record(b)?.kind === "builtin" ? record(b)!.id : undefined);
    const scheme = schemeFor(record(blockData)?.polarity ?? "follow", isLightThemeActive() ? "light" : "dark");
    const swatch = (option: { value: string; label: string; color: string }): ContextMenuItem => ({
        label: option.label,
        swatchColor: option.color,
        checked: pickedId(blockData) === option.value,
        live: () => ({ checked: pickedId(now()) === option.value }),
        keepOpen: true,
        click: () => void setBlockMeta(blockData.oid, { backdrop: builtinRecord(option.value, record(now())?.polarity ?? "follow") } as never),
    });
    const options = builtinOptions(scheme);
    const neutral = (o: { value: string }) => !!slotById(o.value)?.neutral;
    return [
        {
            label: "None",
            checked: !record(blockData),
            live: () => ({ checked: !record(now()) }),
            keepOpen: true,
            click: () => void setBlockMeta(blockData.oid, { backdrop: null } as never),
        },
        { type: "separator" },
        ...options.filter((o) => !neutral(o)).map(swatch),
        { type: "separator" },
        ...options.filter(neutral).map(swatch),
    ];
}

/** The "Backdrop" row of the header menu: the rows above, as a submenu. */
export function backdropSubmenuItem(blockData: Block, live?: LiveBlock): ContextMenuItem {
    return { label: "Backdrop", icon: MENU_ICON.paneColor, type: "submenu", submenu: backdropMenuItems(blockData, live) };
}

/** The palette's "Backdrop": the same rows as a menu on the pane's own frame, as the header menu opens them. */
export function openBackdropMenu(blockId: string): void {
    const frame = document.querySelector<HTMLElement>(`[data-blockid="${blockId}"]`);
    const blockAtom = MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", blockId));
    const blockData = blockAtom();
    if (!frame || !blockData) return;
    const rect = frame.getBoundingClientRect();
    openMenuAt(backdropMenuItems(blockData, blockAtom), { stopPropagation() {}, clientX: rect.left + 12, clientY: rect.top + 36, target: frame });
}
