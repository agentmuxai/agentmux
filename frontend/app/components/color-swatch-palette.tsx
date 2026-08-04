// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { For, Show } from "solid-js";
import type { JSX } from "solid-js";
import "./color-swatch-palette.scss";

interface SwatchColor {
    name: string;
    hex: string;
}

interface ColorSwatchPaletteProps {
    colors: SwatchColor[];
    columns: number;
    currentColor: string | null | undefined;
    onSelect: (hex: string | null) => void;
    showClear?: boolean;
}

export function ColorSwatchPalette(props: ColorSwatchPaletteProps): JSX.Element {
    const showClear = () => props.showClear !== false;

    return (
        <>
            <div
                class="color-swatch-grid"
                style={{ "--swatch-cols": props.columns } as JSX.CSSProperties}
            >
                <For each={props.colors}>
                    {({ name, hex }) => (
                        <div
                            class={clsx("color-swatch", {
                                "color-swatch--selected": (props.currentColor ?? null) === hex,
                            })}
                            title={name}
                            style={{ "background-color": hex }}
                            onClick={() =>
                                props.onSelect((props.currentColor ?? null) === hex ? null : hex)
                            }
                        />
                    )}
                </For>
            </div>
            <Show when={showClear()}>
                <div class="color-swatch-clear">
                    <button onClick={() => props.onSelect(null)}>✕ Clear color</button>
                </div>
            </Show>
        </>
    );
}
