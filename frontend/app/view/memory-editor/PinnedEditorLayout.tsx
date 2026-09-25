// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PinnedEditorLayout — the two-region column every memory editor surface
 * uses (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.4):
 *
 *   top    (flex: 0 1 auto; overflow-y: auto) — header, action bar, name
 *          field, history, diff, hints, errors: everything that used to sit
 *          BELOW an editor;
 *   handle — drag to resize; double-click resets to content-sized;
 *   bottom (flex: 1 1 auto; min-height: min(240px, 50%)) — the editor or
 *          read-only content, filling to the pane's bottom edge.
 *
 * The split is remembered per surface as a FRACTION of the column (so it
 * survives a pane resize), in localStorage under `memoryEditor:split:<key>`,
 * the same best-effort try/catch pattern as the Personal Memory grid's
 * persisted sort (native-memory-manager.tsx). The column must have a
 * definite height (the pane) for the 50% floor to resolve; the floor is what
 * keeps a 128px pane usable.
 */

import { createSignal, onCleanup, type JSX } from "solid-js";
import "./memory-editor.scss";

const STORAGE_PREFIX = "memoryEditor:split:";
const MIN_FRACTION = 0.05;
const MAX_FRACTION = 0.9;
const KEY_STEP = 0.05;

export function splitStorageKey(surface: string): string {
    return `${STORAGE_PREFIX}${surface}`;
}

export function loadStoredSplit(surface: string): number | null {
    try {
        const raw = localStorage.getItem(splitStorageKey(surface));
        if (raw === null) return null;
        const n = Number(raw);
        if (Number.isFinite(n) && n >= MIN_FRACTION && n <= MAX_FRACTION) return n;
    } catch {
        // ignore — fall through to content-sized
    }
    return null;
}

function storeSplit(surface: string, fraction: number | null): void {
    try {
        if (fraction === null) localStorage.removeItem(splitStorageKey(surface));
        else localStorage.setItem(splitStorageKey(surface), String(fraction));
    } catch {
        // best-effort only — the split just won't survive to the next open
    }
}

const clamp = (f: number) => Math.min(MAX_FRACTION, Math.max(MIN_FRACTION, f));

interface PinnedEditorLayoutProps {
    /** Which surface this is — the split is remembered per surface. */
    surface: string;
    top: JSX.Element;
    bottom: JSX.Element;
    class?: string;
    onKeyDown?: (e: KeyboardEvent) => void;
}

export function PinnedEditorLayout(props: PinnedEditorLayoutProps): JSX.Element {
    let root: HTMLDivElement | undefined;
    let topEl: HTMLDivElement | undefined;
    const [fraction, setFraction] = createSignal<number | null>(loadStoredSplit(props.surface));

    const fractionAt = (clientY: number): number | null => {
        const rect = root?.getBoundingClientRect();
        if (!rect || rect.height <= 0) return null;
        return clamp((clientY - rect.top) / rect.height);
    };

    let dragCleanup: (() => void) | null = null;
    const onPointerDown = (e: PointerEvent) => {
        if (e.button !== 0) return;
        e.preventDefault();
        const move = (ev: PointerEvent) => {
            const f = fractionAt(ev.clientY);
            if (f !== null) setFraction(f);
        };
        const up = () => {
            dragCleanup?.();
            storeSplit(props.surface, fraction());
        };
        window.addEventListener("pointermove", move);
        window.addEventListener("pointerup", up);
        dragCleanup = () => {
            window.removeEventListener("pointermove", move);
            window.removeEventListener("pointerup", up);
            dragCleanup = null;
        };
    };
    onCleanup(() => dragCleanup?.());

    // Keyboard-resizable too (a separator is focusable). Starting from
    // content-sized, the first step starts from the top region's real height.
    const onHandleKeyDown = (e: KeyboardEvent) => {
        if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
        e.preventDefault();
        let current = fraction();
        if (current === null) {
            const rootH = root?.getBoundingClientRect().height ?? 0;
            const topH = topEl?.getBoundingClientRect().height ?? 0;
            current = rootH > 0 ? topH / rootH : 0.5;
        }
        const next = clamp(current + (e.key === "ArrowUp" ? -KEY_STEP : KEY_STEP));
        setFraction(next);
        storeSplit(props.surface, next);
    };

    const resetSplit = () => {
        setFraction(null);
        storeSplit(props.surface, null);
    };

    return (
        <div
            ref={root}
            class={`memory-pinned-layout${props.class ? ` ${props.class}` : ""}`}
            data-surface={props.surface}
            onKeyDown={(e) => props.onKeyDown?.(e)}
        >
            <div
                ref={topEl}
                class="memory-pinned-top"
                data-testid="memory-pinned-top"
                style={fraction() !== null ? { height: `${(fraction() as number) * 100}%` } : undefined}
            >
                {props.top}
            </div>
            <div
                class="memory-pinned-handle"
                role="separator"
                aria-orientation="horizontal"
                aria-label="Resize editor"
                aria-valuenow={fraction() !== null ? Math.round((fraction() as number) * 100) : undefined}
                title="Drag to resize · double-click to reset"
                tabIndex={0}
                onPointerDown={onPointerDown}
                onDblClick={resetSplit}
                onKeyDown={onHandleKeyDown}
            />
            <div class="memory-pinned-bottom" data-testid="memory-pinned-bottom">
                {props.bottom}
            </div>
        </div>
    );
}
