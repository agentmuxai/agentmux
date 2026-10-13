// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Resize a terminal's grid and paint it in the same frame.
 *
 * Resizing sets the WebGL canvas's width and height, which empties it, and
 * xterm only asks for the repaint with requestAnimationFrame. Called from a
 * ResizeObserver or animation-frame callback, that repaint runs a frame later,
 * so each column or row change showed one blank terminal frame: the text
 * flickered all through a window drag
 * (docs/analysis/ANALYSIS_TERMINAL_TEXT_FLICKER_ON_WINDOW_RESIZE_2026_10_13.md).
 *
 * Painting reaches into xterm's render service, as the fit sequence copied
 * from FitAddon already did. If a later xterm renames these, the optional
 * calls do nothing and the terminal is back to repainting a frame late.
 */

/** The parts of an xterm `Terminal` (and its internals) this uses. */
export interface PaintableTerminal {
    readonly rows: number;
    readonly cols: number;
    resize(cols: number, rows: number): void;
}

type RenderService = { clear?: () => void; _renderRows?: (start: number, end: number) => void };

function renderService(terminal: PaintableTerminal): RenderService | undefined {
    return (terminal as unknown as { _core?: { _renderService?: RenderService } })._core?._renderService;
}

/** Render every row now, rather than on xterm's next animation frame. */
export function paintRows(terminal: PaintableTerminal): void {
    renderService(terminal)?._renderRows?.(0, terminal.rows - 1);
}

/** Resize to `dims` and paint, if the grid differs. Returns whether it resized. */
export function resizeAndPaint(terminal: PaintableTerminal, dims: { cols: number; rows: number }): boolean {
    if (terminal.rows === dims.rows && terminal.cols === dims.cols) return false;
    renderService(terminal)?.clear?.();
    terminal.resize(dims.cols, dims.rows);
    paintRows(terminal);
    return true;
}
