// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MemoryTile — one tile in a memory tile grid: `{icon, title, badges, meta}`.
 * Generalized out of MemoryFileCard (which is now a thin wrapper) so Armory →
 * Memory → Global renders its entries with exactly the Personal Memory file
 * tile look — same markup, same `memory-file-card*` CSS
 * (native-memory-manager.scss) — per
 * docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3.
 *
 * Interaction contract unchanged from MemoryFileCard: role="button" +
 * Enter/Space (not a native <button>, so a tile can also be a drag source).
 */

import { For, Show, type JSX } from "solid-js";

export interface MemoryTileBadge {
    label: string;
    /** "accent" is the index/lock emphasis; default is the neutral outline. */
    variant?: "accent" | "warning";
    /** Optional Font Awesome icon name shown before the label, e.g. "fa-lock". */
    icon?: string;
    title?: string;
}

export interface MemoryTileProps {
    /** Font Awesome icon name, e.g. "fa-file-lines". */
    icon: string;
    title: string;
    /** File names read best in the mono face; entry names don't. */
    monoTitle?: boolean;
    badges?: MemoryTileBadge[];
    meta?: string;
    onSelect: () => void;
    ariaLabel?: string;
    /** Extra modifier classes, e.g. `{ "memory-file-card--index": true }`. */
    classList?: Record<string, boolean | undefined>;
    testId?: string;
    /** `data-*` attributes (tests and drag-and-drop key off them). */
    data?: Record<string, string>;
    /** Drag-to-reorder hooks; omitted = not draggable. */
    draggable?: boolean;
    onDragStart?: (e: DragEvent) => void;
    onDragOver?: (e: DragEvent) => void;
    onDragLeave?: (e: DragEvent) => void;
    onDrop?: (e: DragEvent) => void;
    onDragEnd?: (e: DragEvent) => void;
}

export function MemoryTile(props: MemoryTileProps): JSX.Element {
    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            props.onSelect();
        }
    };

    const dataAttrs = () =>
        Object.fromEntries(Object.entries(props.data ?? {}).map(([k, v]) => [`data-${k}`, v]));

    return (
        <div
            class="memory-file-card"
            classList={props.classList ?? {}}
            role="button"
            tabIndex={0}
            onClick={() => props.onSelect()}
            onKeyDown={handleKeyDown}
            data-testid={props.testId ?? "memory-tile"}
            aria-label={props.ariaLabel ?? [props.title, props.meta].filter(Boolean).join(" — ")}
            draggable={props.draggable ? true : undefined}
            onDragStart={(e) => props.onDragStart?.(e)}
            onDragOver={(e) => props.onDragOver?.(e)}
            onDragLeave={(e) => props.onDragLeave?.(e)}
            onDrop={(e) => props.onDrop?.(e)}
            onDragEnd={(e) => props.onDragEnd?.(e)}
            {...dataAttrs()}
        >
            <i class={`memory-file-card-icon fa-sharp fa-solid ${props.icon}`} aria-hidden="true" />
            <span class="memory-file-card-info">
                <span
                    class="memory-file-card-title"
                    classList={{ "memory-file-card-title--plain": props.monoTitle === false }}
                    title={props.title}
                >
                    {props.title}
                </span>
                <span class="memory-file-card-badges">
                    <For each={props.badges ?? []}>
                        {(badge) => (
                            <span
                                class="memory-file-card-badge"
                                classList={{
                                    "memory-file-card-badge--index": badge.variant === "accent",
                                    "memory-file-card-badge--warning": badge.variant === "warning",
                                }}
                                title={badge.title}
                            >
                                <Show when={badge.icon}>
                                    <i class={`fa-sharp fa-solid ${badge.icon}`} aria-hidden="true" />{" "}
                                </Show>
                                {badge.label}
                            </span>
                        )}
                    </For>
                </span>
                <Show when={props.meta}>
                    <span class="memory-file-card-meta">{props.meta}</span>
                </Show>
            </span>
        </div>
    );
}
