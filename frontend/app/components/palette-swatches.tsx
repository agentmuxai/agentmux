// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The one colour picker: the shared palette's 16 slots as a grid of swatches
 * (docs/specs/SPEC_SHARED_PERCEPTUAL_PALETTE_16_2026_11_01.md §5.4). Used by
 * the window-tab menu, Settings > Widget colors and the browser profile
 * pickers. Each swatch is painted in the identity tone of the current theme,
 * what the ring and the underline will be, so what you click is what you get.
 * A name on hover and as the accessible label, and a check mark on the chosen
 * one, so the picker never depends on colour alone.
 */

import clsx from "clsx";
import { For, Show, type JSX } from "solid-js";

import { paneRoleColor } from "@/app/block/pane-color-scheme";
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
    /** Swatches per row. Default 8: two rows of the 16. */
    columns?: number;
    class?: string;
}

export function PaletteSwatches(p: PaletteSwatchesProps): JSX.Element {
    const chosen = () => (p.value === null || p.value === undefined ? undefined : slotOf(p.value)?.id);
    const tone = (slot: PaletteSlot) => paneRoleColor(undefined, slot.id, isLightThemeActive(), "identity");
    return (
        <div
            class={clsx("palette-swatches", p.class)}
            role="group"
            aria-label={p.label}
            style={{ "--palette-cols": p.columns ?? 8 } as JSX.CSSProperties}
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
            <For each={PALETTE}>
                {(slot) => (
                    <Tooltip content={slot.name} placement="top" divClassName="palette-swatch-anchor">
                        <Button
                            class="palette-swatch"
                            style={{ "background-color": tone(slot) }}
                            pressed={chosen() === slot.id}
                            aria-label={slot.name}
                            onClick={() => p.onSelect(slot)}
                        />
                    </Tooltip>
                )}
            </For>
        </div>
    );
}
