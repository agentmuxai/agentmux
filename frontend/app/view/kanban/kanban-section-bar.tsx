// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The narrow Kanban's bar of links to its sections, with Collapse all: one link per section in the order the list
 * draws them, the stage's name, its count and a mark in its colour, and the section you are in marked as current. It
 * sits at the top of the list and stays there while the list scrolls. A link scrolls that section's header to the
 * top and opens the section if it is folded; Collapse all folds every section, and Expand all opens them again.
 */

import { createEffect, createSignal, For, onCleanup, onMount, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";

import type { KanbanStage } from "./kanban-stages";

import "./kanban-section-bar.scss";

/** One link of the bar. */
export interface SectionBarEntry {
    id: string;
    name: string;
    /** The stage's colour name (`data-stage-colour`), which gives the mark its colour. */
    colour: string;
    count: number;
}

/**
 * The sections in the order the narrow list draws them: the stage that shows what needs the operator first (kanban.scss
 * puts it at the head of the list), the others in the board's order.
 */
export function sectionOrder<T extends Pick<KanbanStage, "takes">>(stages: readonly T[]): T[] {
    const needs = (s: T) => s.takes.includes("needs");
    return [...stages.filter(needs), ...stages.filter((s) => !needs(s))];
}

/** "go to Merged, 3 tasks": what a screen reader says for a link. */
export function sectionLinkLabel(entry: Pick<SectionBarEntry, "name" | "count">): string {
    return t("kanban.bar.goTo", { name: entry.name, count: entry.count });
}

/**
 * The section you are in: the last whose top has reached the line under the bar, or the first when none has.
 * `tops` are the sections' tops in list order (distance from the top of the list's content), `scrollTop` how far the
 * list is scrolled and `barHeight` what the bar covers.
 */
export function currentSection(tops: readonly number[], scrollTop: number, barHeight: number): number {
    let current = 0;
    for (let i = 0; i < tops.length; i++) {
        if (tops[i] <= scrollTop + barHeight + 1) current = i;
    }
    return current;
}

export interface SectionBarProps {
    entries: readonly SectionBarEntry[];
    /** Every section is folded: the button then opens them all. */
    allFolded: boolean;
    onGo: (id: string) => void;
    onToggleAll: () => void;
    /** The id of the section you are in, or null. */
    current: string | null;
    /** The bar's own element, for its height and the list that scrolls under it. */
    bindBar?: (el: HTMLElement) => void;
}

export function KanbanSectionBar(props: SectionBarProps): JSX.Element {
    return (
        <nav class="kanban-section-bar" aria-label={t("kanban.bar.label")} ref={(el) => props.bindBar?.(el)}>
            <div class="kanban-section-bar__links">
                <For each={props.entries}>
                    {(entry) => (
                        <a
                            class="kanban-section-bar__link"
                            classList={{ "kanban-section-bar__link--current": props.current === entry.id }}
                            data-stage-colour={entry.colour}
                            href={`#kanban-section-${entry.id}`}
                            aria-label={sectionLinkLabel(entry)}
                            aria-current={props.current === entry.id ? "location" : undefined}
                            onClick={(e) => {
                                e.preventDefault();
                                props.onGo(entry.id);
                            }}
                        >
                            <span class="kanban-section-bar__name">{entry.name}</span>
                            <span class="kanban-section-bar__count">{entry.count}</span>
                        </a>
                    )}
                </For>
            </div>
            <Button density="compact" tone="quiet" class="kanban-section-bar__all" onClick={() => props.onToggleAll()}>
                {props.allFolded ? t("kanban.bar.expandAll") : t("kanban.bar.collapseAll")}
            </Button>
        </nav>
    );
}

/**
 * Follow the list that scrolls under the bar: which section is current, and the bar's height as a variable the sections'
 * own sticky heads sit below (`--kanban-bar-h`). `board` is the list's element, `sectionEl` finds a section's element and
 * `enabled` says whether the list has a bar now; without one nothing is measured and no variable is set.
 */
export function useSectionBar(
    board: () => HTMLElement | undefined,
    ids: () => readonly string[],
    sectionEl: (id: string) => HTMLElement | null,
    enabled: () => boolean
) {
    const [current, setCurrent] = createSignal<string | null>(null);
    let bar: HTMLElement | undefined;
    // A link that cannot bring its section to the top (the list is at its end and the section is short) still makes that
    // section the current one, until the person scrolls the list themselves.
    let pinned: string | null = null;
    let observer: ResizeObserver | undefined;
    const barHeight = () => bar?.offsetHeight ?? 0;
    const update = () => {
        const list = board();
        const order = ids();
        if (!list || !bar || order.length === 0) return setCurrent(null);
        const tops = order.map((id) => sectionEl(id)?.offsetTop ?? 0);
        const natural = currentSection(tops, list.scrollTop, barHeight());
        // The pin ends when the scroll has brought its section to the line, and with it nothing is held.
        if (pinned && order.indexOf(pinned) <= natural) pinned = null;
        setCurrent((pinned ?? order[natural]) || null);
    };
    const pin = (id: string) => {
        pinned = id;
        update();
    };
    const unpin = () => {
        if (!pinned) return;
        pinned = null;
        update();
    };
    const measure = () => {
        board()?.style.setProperty("--kanban-bar-h", `${barHeight()}px`);
        update();
    };
    onMount(() => {
        const list = board();
        if (!list) return;
        list.addEventListener("scroll", update, { passive: true });
        // The person's own scrolling, not the link's: it ends a pin.
        const own = ["wheel", "touchstart", "pointerdown", "keydown"] as const;
        own.forEach((type) => list.addEventListener(type, unpin, { passive: true }));
        observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(measure);
        if (bar) observer?.observe(bar);
        observer?.observe(list);
        onCleanup(() => {
            list.removeEventListener("scroll", update);
            own.forEach((type) => list.removeEventListener(type, unpin));
            observer?.disconnect();
        });
    });
    // The bar comes and goes with the narrow layout: measure it when it is drawn, clear what it set when it is not.
    createEffect(() => {
        if (enabled()) return;
        bar = undefined;
        board()?.style.removeProperty("--kanban-bar-h");
        setCurrent(null);
    });
    return {
        current,
        update,
        pin,
        bindBar: (el: HTMLElement) => {
            bar = el;
            observer?.observe(el);
            measure();
        },
    };
}
