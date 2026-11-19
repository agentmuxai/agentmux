// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The one colour picker: the shared palette's 14 slots as a grid of swatches,
 * the 12 hues and then Black and White
 * (docs/specs/SPEC_SHARED_PALETTE_12_HUES_BLACK_WHITE_2026_11_16.md §7). Used
 * by the window-tab menu, Settings > Widget colors and the browser profile
 * pickers. A hue's swatch is painted in the identity tone of the current
 * theme, what the ring and the underline will be, so what you click is what
 * you get; Black and White are painted as themselves. A hairline around every
 * swatch, so a black one shows on a dark menu and a white one on a light
 * menu. A name on hover and as the accessible label, and a check mark on the
 * chosen one, so the picker never depends on colour alone.
 */

import clsx from "clsx";
import { For, Show, type JSX } from "solid-js";

import { slotSwatchColor } from "@/app/block/pane-color-scheme";
import { isLightThemeActive } from "@/app/block/pane-identity";
import { PALETTE, slotOf, type PaletteSlot } from "@/app/theme/palette";
import { Button } from "@/element/ui";
import { Tooltip } from "../element/tooltip";
import "./palette-swatches.scss";

export interface PaletteSwatchesProps {
    /** What is chosen now: a slot id, a stored hex or a legacy hue, which is read as its slot. `null`: "none" is chosen. */
    value: string | number | null | undefined;
    /** A slot was picked, or, with `null`, the "none" swatch. */
    onSelect: (slot: PaletteSlot | null) => void;
    /** Names the group for assistive technology. */
    label: string;
    /** Put a leading swatch that means "no colour" (its name, e.g. "No colour", "Default") in the grid. */
    noneLabel?: string;
    /** Columns, fixed, the "none" swatch's own included. Default: two rows of 7 colours; with the "none" swatch it is
     *  alone in the first column and the colours are two rows of 7 beside it (8 columns). A context that must narrow
     *  the grid with its pane's width leaves this out and sets `--palette-cols` in its own styles. */
    columns?: number;
    /** What the swatch of a slot shows when picking it paints a fill with a bar (a window tab): that fill and bar, in the
     *  theme now, the same call the surface itself draws from. Without it a swatch is the slot's identity colour. */
    sample?: (slot: PaletteSlot) => { fill: string; bar: string };
    class?: string;
}

export function PaletteSwatches(p: PaletteSwatchesProps): JSX.Element {
    const chosen = () => (p.value === null || p.value === undefined ? undefined : slotOf(p.value)?.id);
    // The check mark's colour. On a hue it is the theme's background; that is black on the black swatch of a dark
    // theme and white on the white swatch of a light one, so a neutral names its own: the other neutral's colour.
    const swatchStyle = (slot: PaletteSlot): JSX.CSSProperties => {
        if (p.sample) {
            const { fill, bar } = p.sample(slot);
            // On the fill the mark takes the theme's text, as the tab's label does.
            return { "--swatch-fill": fill, "--swatch-bar": bar, "--palette-swatch-ink": "var(--main-text-color)" } as JSX.CSSProperties;
        }
        return {
            "background-color": slotSwatchColor(slot, isLightThemeActive()),
            ...(slot.neutral && { "--palette-swatch-ink": PALETTE.find((s) => s.neutral && s.neutral !== slot.neutral)?.hex }),
        };
    };
    return (
        <div
            class={clsx("palette-swatches", p.noneLabel && "palette-swatches--with-none", p.class)}
            role="group"
            aria-label={p.label}
            style={p.columns === undefined ? undefined : ({ "--palette-cols": p.columns } as JSX.CSSProperties)}
        >
            <Show when={p.noneLabel}>
                {(label) => (
                    <Tooltip content={label()} placement="top" divClassName="palette-swatch-anchor">
                        <Button
                            class="palette-swatch palette-swatch--none"
                            icon="ban"
                            pressed={p.value === null}
                            aria-label={label()}
                            onClick={() => p.onSelect(null)}
                        />
                    </Tooltip>
                )}
            </Show>
            <div class="palette-swatches__colours">
                <For each={PALETTE}>
                    {(slot) => (
                        <Tooltip content={slot.name} placement="top" divClassName="palette-swatch-anchor">
                            <Button
                                class={clsx("palette-swatch", p.sample && "palette-swatch--sample")}
                                style={swatchStyle(slot)}
                                pressed={chosen() === slot.id}
                                aria-label={slot.name}
                                onClick={() => p.onSelect(slot)}
                            />
                        </Tooltip>
                    )}
                </For>
            </div>
        </div>
    );
}
