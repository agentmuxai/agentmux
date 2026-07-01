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

    const style = (): JSX.CSSProperties => {
        // Panel is ~180px wide × ~160px tall (4-col × 3-row swatch + sep + clear).
        // Clamp so it never renders off the right or bottom edge of the viewport.
        const PANEL_W = 188;
        const PANEL_H = 168;
        const vw = window.innerWidth;
        const vh = window.innerHeight;
        const rawLeft = props.anchor.left;
        const rawTop = props.anchor.bottom + 4;
        const left = Math.min(rawLeft, vw - PANEL_W - 4);
        const top = rawTop + PANEL_H > vh ? props.anchor.top - PANEL_H - 4 : rawTop;
        return {
            position: "fixed",
            top: `${Math.max(4, top)}px`,
            left: `${Math.max(4, left)}px`,
            "z-index": 9999,
        };
    };

    // Map hue to swatch preview hex for currentColor comparison.
    // If the stored hue doesn't match any current swatch (retired hue from an
    // older palette), fall back to the nearest swatch so one is shown selected.
    const currentPreview = (): string | null => {
        if (props.currentHue == null) return null;
        const exact = PANE_SWATCH_COLORS.find((s) => s.hue === props.currentHue);
        if (exact) return exact.preview;
        // Nearest hue by circular distance (mod 360)
        const nearest = PANE_SWATCH_COLORS.reduce((best, s) => {
            const d = Math.min(
                Math.abs(s.hue - props.currentHue!),
                360 - Math.abs(s.hue - props.currentHue!)
            );
            const bd = Math.min(
                Math.abs(best.hue - props.currentHue!),
                360 - Math.abs(best.hue - props.currentHue!)
            );
            return d < bd ? s : best;
        });
        return nearest.preview;
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
