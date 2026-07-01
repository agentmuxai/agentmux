// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ColorSwatchPalette } from "@/app/components/color-swatch-palette";
import { onCleanup, onMount } from "solid-js";
import { Portal } from "solid-js/web";
import type { JSX } from "solid-js";
import { PANE_SWATCH_COLORS, setHue } from "./pane-color-menu";
import "./pane-color-panel.scss";

interface PaneColorPanelProps {
    anchor: DOMRect;
    currentHue: number | null;
    blockId: string;
    onClose: () => void;
}

export function PaneColorPanel(props: PaneColorPanelProps): JSX.Element {
    let panelRef!: HTMLDivElement;

    onMount(() => {
        const handleClickOutside = (e: MouseEvent) => {
            if (panelRef && !panelRef.contains(e.target as Node)) {
                props.onClose();
            }
        };
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === "Escape") props.onClose();
        };
        // CEF native context menus steal OS focus while open and do not fire DOM
        // mousedown when an item is selected. Listening for window focus returning
        // catches the close of the native menu (regardless of whether the user
        // picked an item or dismissed it) so the swatch panel doesn't linger.
        const handleWindowFocus = () => props.onClose();
        document.addEventListener("mousedown", handleClickOutside);
        document.addEventListener("keydown", handleKeyDown);
        window.addEventListener("focus", handleWindowFocus);
        onCleanup(() => {
            document.removeEventListener("mousedown", handleClickOutside);
            document.removeEventListener("keydown", handleKeyDown);
            window.removeEventListener("focus", handleWindowFocus);
        });
    });

    const style = (): JSX.CSSProperties => ({
        position: "fixed",
        top: `${props.anchor.bottom + 4}px`,
        left: `${props.anchor.left}px`,
        "z-index": 9999,
    });

    // Map hue to swatch preview hex for currentColor comparison
    const currentPreview = (): string | null => {
        if (props.currentHue == null) return null;
        return PANE_SWATCH_COLORS.find((s) => s.hue === props.currentHue)?.preview ?? null;
    };

    const swatchColors = PANE_SWATCH_COLORS.map((s) => ({ name: s.label, hex: s.preview }));

    const handleSelect = (hex: string | null) => {
        if (hex == null) {
            setHue(props.blockId, null);
        } else {
            const match = PANE_SWATCH_COLORS.find((s) => s.preview === hex);
            if (match) setHue(props.blockId, match.hue);
        }
        props.onClose();
    };

    return (
        <Portal>
            <div ref={panelRef!} class="pane-color-panel" style={style()} data-pane-overlay>
                <div class="pane-color-panel-sep" />
                <ColorSwatchPalette
                    colors={swatchColors}
                    columns={4}
                    currentColor={currentPreview()}
                    onSelect={handleSelect}
                    showClear={false}
                />
                <div class="pane-color-panel-clear">
                    <button onClick={() => handleSelect(null)}>✕ Clear color</button>
                </div>
            </div>
        </Portal>
    );
}
