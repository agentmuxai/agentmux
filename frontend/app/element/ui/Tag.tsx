// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { createContext, createSignal, createUniqueId, For, type JSX, Show, splitProps, useContext } from "solid-js";

import { Tooltip } from "@/app/element/tooltip";
import { TAG_GROUPS, type TagGroup, type TagTone } from "./tag-tokens";

import "./tag.scss";

export type { TagGroup, TagTone } from "./tag-tokens";
export { TAG_GROUPS } from "./tag-tokens";

/** `chip`: a property, inline. `ribbon`: what a click on the card will do, on its corner. `count`: a number on a corner. */
export type TagKind = "chip" | "ribbon" | "count";
export type TagCorner = "top-right" | "bottom-right" | "top-left" | "bottom-left";

export interface TagProps {
    /** Default `chip`. */
    kind?: TagKind;
    /** Default `neutral`. */
    tone?: TagTone;
    /** `sm` is `--text-xs`, `xs` is `--text-2xs`. Default `sm` (`xs` for a ribbon or a count). */
    size?: "sm" | "xs";
    /** A Font Awesome solid icon name without its prefix: "download", "circle-up". Decorative: the words carry the meaning. */
    icon?: string;
    /** Where a ribbon or a count sits on its card (a positioned ancestor). Default `bottom-right` for a ribbon, `top-right` for a count. */
    corner?: TagCorner;
    /**
     * Which place in a card's tags this one takes, from `TAG_GROUPS`. Read by `TagList`, which
     * orders by it; a lone tag ignores it.
     */
    group?: TagGroup;
    /**
     * Longer text for a chip: up to about four short lines, shown in the shared tooltip on hover,
     * focus and tap. Not interactive (no links or buttons: a tooltip closes when the pointer or
     * focus leaves). A chip with a detail takes focus; one without does not.
     */
    detail?: JSX.Element;
    /** A native tooltip, for a chip with no `detail` (kept from `Pill`). */
    title?: string;
    class?: string;
    children: JSX.Element;
}

/**
 * A tag: a small label on a card or in a row. Built on `Pill`'s tones. The wash is opaque (the
 * tone laid over the surface the tag sits on), because a ribbon or a count sits on the card's
 * corner over its text; the outline is the full tone colour, which every theme keeps at 3:1
 * against the surface, and the text is the main text colour at 4.5:1 on the wash
 * (tag-contrast.test.ts). Status is carried by the words and the icon as well as the colour.
 * docs/specs/SPEC_UI_PANEL_PRIMITIVE_2026_10_29.md §9.
 */
export function Tag(props: TagProps): JSX.Element {
    const kind = () => props.kind ?? "chip";
    const size = () => props.size ?? (kind() === "chip" ? "sm" : "xs");
    const corner = () => props.corner ?? (kind() === "count" ? "top-right" : "bottom-right");
    const hasDetail = () => kind() === "chip" && props.detail != null;
    const classes = () =>
        clsx(
            "ui-tag",
            `ui-tag--${kind()}`,
            `ui-tag--${props.tone ?? "neutral"}`,
            `ui-tag--${size()}`,
            kind() !== "chip" && `ui-tag--${corner()}`,
            props.class,
        );
    const body = (): JSX.Element => (
        <>
            <Show when={props.icon}>
                <i class={`fa-solid fa-${props.icon}`} aria-hidden="true" />
            </Show>
            {props.children}
        </>
    );
    return (
        <Show when={hasDetail()} fallback={<span class={classes()} title={props.title}>{body()}</span>}>
            <DetailedChip class={classes()} detail={props.detail}>
                {body()}
            </DetailedChip>
        </Show>
    );
}

/**
 * A chip whose `detail` shows in the shared tooltip. That tooltip opens on hover only, so focus
 * and tap are added here: the chip is focusable, the tooltip stays open while it has focus or
 * after a tap, and Escape closes it. The description is also in the page as hidden text with an
 * id, because the tooltip's own content is not in the DOM while it is closed.
 */
function DetailedChip(props: { class: string; detail: JSX.Element; children: JSX.Element }): JSX.Element {
    const [pinned, setPinned] = createSignal(false);
    // Escape dismisses the detail even while the pointer is still over the chip, until the pointer
    // leaves or the chip is touched again.
    const [dismissed, setDismissed] = createSignal(false);
    const descId = `ui-tag-detail-${createUniqueId()}`;
    return (
        <Tooltip
            content={<div class="ui-tag-detail">{copyOf(props.detail)}</div>}
            placement="top"
            forceOpen={pinned()}
            disable={dismissed()}
            divClassName="ui-tag-anchor"
        >
            <span
                class={props.class}
                tabindex="0"
                aria-describedby={descId}
                onFocus={() => {
                    setDismissed(false);
                    setPinned(true);
                }}
                onBlur={() => {
                    setDismissed(false);
                    setPinned(false);
                }}
                onMouseLeave={() => setDismissed(false)}
                // A tap focuses first and clicks second, so a click only pins; blur or Escape unpins.
                onClick={() => {
                    setDismissed(false);
                    setPinned(true);
                }}
                onKeyDown={(e) => {
                    if (e.key === "Escape" && pinned()) {
                        e.stopPropagation();
                        setPinned(false);
                        setDismissed(true);
                    }
                }}
            >
                {props.children}
            </span>
            {/* Outside the chip, so the detail is read once (as its description), not also as its name. */}
            <span id={descId} class="ui-tag-sr">
                {props.detail}
            </span>
        </Tooltip>
    );
}

/**
 * A second copy of `detail` for the tooltip. The page keeps the original as the hidden description, and a
 * DOM node given to two places is moved, not copied, so opening the tooltip would empty the description.
 */
function copyOf(detail: JSX.Element): JSX.Element {
    if (typeof Node !== "undefined" && detail instanceof Node) return detail.cloneNode(true) as unknown as JSX.Element;
    if (Array.isArray(detail)) return detail.map(copyOf);
    return detail;
}

// ── TagList ───────────────────────────────────────────────────────────────────

export interface TagSpec {
    /** Stable key. */
    id: string;
    label: JSX.Element;
    kind?: "chip" | "ribbon";
    tone?: TagTone;
    icon?: string;
    group?: TagGroup;
    detail?: JSX.Element;
    title?: string;
}

export interface TagListProps {
    tags: readonly TagSpec[];
    /** Most chips shown before the rest fold into "+N". The ribbon is not counted. Default 3. */
    max?: number;
    size?: "sm" | "xs";
    class?: string;
}

/** A tag's rank: its group's, or `fact` (last) when it names none. */
const rankOf = (tag: TagSpec): number => TAG_GROUPS[tag.group ?? "fact"];

/**
 * The chips of a card in the fixed order of `TAG_GROUPS` (then the order given), cut to `max`
 * with "+N" for the rest; the "+N" lists the hidden ones in its tooltip. Cutting from the end
 * keeps what needs attention. A ribbon is not a chip: it is rendered apart, on its corner, and
 * never folded in.
 */
export function TagList(props: TagListProps): JSX.Element {
    const ribbons = () => props.tags.filter((t) => t.kind === "ribbon");
    const chips = () =>
        props.tags
            .map((tag, i) => ({ tag, i }))
            .filter(({ tag }) => tag.kind !== "ribbon")
            .sort((a, b) => rankOf(a.tag) - rankOf(b.tag) || a.i - b.i)
            .map(({ tag }) => tag);
    const max = () => Math.max(0, props.max ?? 3);
    const shown = () => chips().slice(0, max());
    const hidden = () => chips().slice(max());
    return (
        <>
            <For each={ribbons()}>
                {(t) => (
                    <Tag kind="ribbon" tone={t.tone} icon={t.icon} size={props.size === "sm" ? "sm" : undefined}>
                        {t.label}
                    </Tag>
                )}
            </For>
            <Show when={chips().length > 0}>
                <span class={clsx("ui-tag-list", props.class)}>
                    <For each={shown()}>
                        {(t) => (
                            <Tag tone={t.tone} icon={t.icon} size={props.size} detail={t.detail} title={t.title}>
                                {t.label}
                            </Tag>
                        )}
                    </For>
                    <Show when={hidden().length > 0}>
                        <Tag
                            size={props.size}
                            detail={
                                <ul class="ui-tag-more">
                                    <For each={hidden()}>{(t) => <li>{t.label}</li>}</For>
                                </ul>
                            }
                        >
                            {`+${hidden().length}`}
                        </Tag>
                    </Show>
                </span>
            </Show>
        </>
    );
}

// ── FilterChip ────────────────────────────────────────────────────────────────

interface FilterChipsContextValue {
    isPressed: (id: string) => boolean;
    toggle: (id: string) => void;
}

const FilterChipsContext = createContext<FilterChipsContextValue>();

export interface FilterChipsProps {
    /** The ids of the chips that are on. */
    value: readonly string[];
    onChange: (next: string[]) => void;
    /** Names the group for assistive tech ("Filter agents"). */
    "aria-label": string;
    class?: string;
    children: JSX.Element;
}

/**
 * A group of toggle chips that narrow a grid: any number can be on. One-of-N is
 * `SegmentedControl`'s job, so there is no single-select mode here.
 */
export function FilterChips(props: FilterChipsProps): JSX.Element {
    const ctx: FilterChipsContextValue = {
        isPressed: (id) => props.value.includes(id),
        toggle: (id) =>
            props.onChange(props.value.includes(id) ? props.value.filter((v) => v !== id) : [...props.value, id]),
    };
    return (
        <FilterChipsContext.Provider value={ctx}>
            <div role="group" aria-label={props["aria-label"]} class={clsx("ui-filter-chips", props.class)}>
                {props.children}
            </div>
        </FilterChipsContext.Provider>
    );
}

export interface FilterChipProps {
    /** The id `FilterChips` keeps in its `value` while this chip is on. */
    id: string;
    /** How many items this chip would leave ("Free 7"); live, as the other filters change. */
    count?: number;
    tone?: TagTone;
    icon?: string;
    disabled?: boolean;
    class?: string;
    children: JSX.Element;
}

/** A tag that is a toggle: quiet at rest, the tone's line and wash while on. */
export function FilterChip(props: FilterChipProps): JSX.Element {
    const [local] = splitProps(props, ["id", "count", "tone", "icon", "disabled", "class", "children"]);
    const ctx = useContext(FilterChipsContext);
    if (!ctx) throw new Error("FilterChip must be inside FilterChips");
    const pressed = () => ctx.isPressed(local.id);
    return (
        <button
            type="button"
            class={clsx("ui-tag", "ui-tag--chip", "ui-tag--sm", `ui-tag--${local.tone ?? "accent"}`, "ui-filter-chip", local.class)}
            aria-pressed={pressed()}
            disabled={local.disabled}
            onClick={() => ctx.toggle(local.id)}
        >
            <Show when={local.icon}>
                <i class={`fa-solid fa-${local.icon}`} aria-hidden="true" />
            </Show>
            {local.children}
            <Show when={local.count != null}>
                <span class="ui-filter-chip-count">{local.count}</span>
            </Show>
        </button>
    );
}
