// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCard — a definition tile in the agent picker.
 *
 * Per SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23 the card is driven
 * by the CLI catalog when one exists for the provider — the title is
 * the CLI brand name ("Claude Code") and the capability blurb
 * ("Anthropic's coding agent") sits as a caption below. Providers
 * without a catalog entry fall back to the AgentDefinition row's own
 * description/name fields.
 *
 * Clicking the body opens the template's new-agent panel under its
 * row (starting with the install if the CLI isn't installed yet).
 * When `installed === false`, a
 * bottom-right "Click to install" ribbon is overlaid on the card —
 * like a "For Sale" sign. The ribbon disappears once installation
 * succeeds (the picker re-runs install.check when the panel's install
 * ends); while it runs the ribbon reads "Installing…".
 */

import { createMemo, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { TagList, type TagSpec } from "@/app/element/ui";
import { focusManager } from "@/app/store/focusManager";
import { ProviderLogo } from "@/element/ProviderLogo";
import { getCliCatalogEntry } from "../defaults/cli-catalog";
import { accessTagsFor } from "../defaults/provider-access-model";
import { AccessTagDetail } from "./AccessTagDetail";
import type { PinDrift } from "../providers/version-drift";
import type { AgentDefinition } from "@/app/store/rpc-api";

interface AgentCardProps {
    agent: AgentDefinition;
    launching: boolean;
    disabled: boolean;
    /** undefined = not yet checked.
     *  true = the CLI was found (AgentMux's install, or the user's own copy of
     *  a CLI AgentMux doesn't install).
     *  false = not found — render the bottom-right ribbon. */
    installed: boolean | undefined;
    /** Its CLI is being installed (the run outlives its panel): the ribbon says so. */
    installing?: boolean;
    /** Its new-agent panel is open below its row. */
    expanded?: boolean;
    /**
     * How the installed CLI relates to the version AgentMux pins. Only
     * `"behind-pin"` renders anything; every other state (including
     * `"unknown"`, which is what a provider with no pin or no readable version
     * resolves to) is deliberately silent. A card is a launch affordance, not
     * a version dashboard — the Toolchain pane is where the full picture
     * lives. See `../providers/version-drift.ts`.
     */
    drift?: PinDrift;
    /**
     * Opens or closes this template's new-agent panel.
     * The synthetic `MouseEvent` is forwarded so the parent can read
     * modifier keys (Shift/Ctrl/Alt).
     */
    onLaunch: (agent: AgentDefinition, evt?: MouseEvent | KeyboardEvent) => void;
    /**
     * Option E (PR 2 of 2): when true, this card's agent has a
     * non-empty session zone (`agent:<defId>:current`). Renders a small
     * "+ New" secondary button for a fresh start. The picker passes
     * `false` for every template today.
     */
    hasCurrentSession?: boolean;
    /** Option E: invoked when the user clicks the "+ New" affordance.
     *  Parent archives the current zone and starts a fresh session. */
    onNewSession?: (agent: AgentDefinition) => void;
    /**
     * Phase 2 (Q2 Decision Y — hide templates): right-click handler.
     * Currently only the templates tier wires this up so the user can
     * hide a template. My-agent rows leave it undefined and the card
     * falls back to the browser default context menu.
     */
    onContextMenu?: (agent: AgentDefinition, evt: MouseEvent) => void;
    /** When true this card is the picker's default choice (the
     *  most-recently-used agent) — focus it on mount so Enter launches
     *  it and the focus ring marks it as the default. */
    defaultFocus?: boolean;
    /** The pane this card renders in. Scopes `defaultFocus` to "only if this
     *  is the selected pane and the user isn't already typing in it". */
    blockId?: string;
}

/** Below this width a card shows two chips and its corner action drops to its own line. */
const NARROW_CARD_PX = 300;

export const AgentCard = (props: AgentCardProps): JSX.Element => {
    const catalog = createMemo(() => getCliCatalogEntry(props.agent.provider));
    const title = () => catalog()?.displayName || props.agent.name;
    const caption = () => catalog()?.blurb || props.agent.description || props.agent.name;

    // The card's tags (docs/specs/SPEC_PROVIDER_CARD_TAGS_FREE_LOCAL_2026_10_29.md §3):
    // the ribbon is what a click will do; the chips, in the kit's fixed order, are
    // properties: Free and Local from the access data, and a newer pinned CLI.
    const tags = createMemo<TagSpec[]>(() => {
        const out: TagSpec[] = [];
        if (props.installed === false) {
            out.push({
                id: "install",
                kind: "ribbon",
                tone: "ok",
                icon: "download",
                label: props.installing ? "Installing…" : "Click to install",
            });
        }
        const now = new Date();
        for (const tag of accessTagsFor(props.agent.provider, now)) {
            out.push({
                id: tag.label.toLowerCase(),
                label: tag.label,
                tone: tag.tone,
                icon: tag.icon,
                group: tag.label === "Free" ? "plan" : "where",
                detail: <AccessTagDetail tag={tag} />,
            });
        }
        // Not alongside the install ribbon: "not installed" already supersedes "out of
        // date", and two calls to action on one card is what the ribbon's spec says not to do.
        if (props.installed !== false && props.drift === "behind-pin") {
            out.push({
                id: "update",
                label: "Update available",
                tone: "warn",
                icon: "circle-up",
                group: "fact",
                title: "A newer CLI version is pinned by AgentMux — update it in the Toolchain pane",
            });
        }
        return out;
    });
    const chipCount = createMemo(() => tags().filter((t) => t.kind !== "ribbon").length);
    // A corner action (the ribbon, or "+ New") the card shows now.
    const hasAction = () => props.installed === false || (!!props.hasCurrentSession && !props.launching);

    // A narrow card shows two chips, the rest folding into "+N", and its corner action
    // moves to its own line under them (`agent-card--narrow` in _picker.scss).
    const [narrow, setNarrow] = createSignal(false);
    let cardEl: HTMLDivElement | undefined;
    onMount(() => {
        if (typeof ResizeObserver === "undefined" || !cardEl) return;
        const observer = new ResizeObserver((entries) => {
            const width = entries[0]?.contentRect.width ?? 0;
            if (width > 0) setNarrow(width < NARROW_CARD_PX);
        });
        observer.observe(cardEl);
        onCleanup(() => observer.disconnect());
    });
    onMount(() => {
        if (!props.defaultFocus || props.disabled || !props.blockId) return;
        // Through the shared guard, never a bare focus(): this used to take
        // the caret from whichever pane the user was typing in, and scroll the
        // picker down to the "New Agent" header — on every `agents:changed`
        // refetch, since those used to remount the card
        // (REPORT_AGENT_PANE_SIDE_BY_SIDE_SCROLL_AND_FOCUS_QUIRKS_2026_09_23.md §2).
        focusManager.claimFocusOnMount(props.blockId, () => {
            cardEl?.focus({ preventScroll: true });
            return true;
        });
    });

    // A press on a chip (to read its tooltip) is the chip's own, not the card's: it must not
    // open or close the creation panel.
    const onChip = (e: Event) => e.target instanceof Element && e.target.closest(".agent-card-tags") !== null;

    const handleCardClick = (e: MouseEvent) => {
        if (onChip(e)) return;
        if (!props.disabled) props.onLaunch(props.agent, e);
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (onChip(e)) return;
        if (typeToFilter(e)) return;
        if (props.disabled) return;
        if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            props.onLaunch(props.agent, e);
        }
    };

    // Type to filter: the picker focuses its first card (Enter launches it),
    // but a printable key goes into the pane's filter box, so landing on the
    // picker after a close still lets the user type right away.
    // SPEC_FOCUS_FOLLOWS_SELECTION_2026_10_08.md §2d.
    const typeToFilter = (e: KeyboardEvent): boolean => {
        const filter = () => cardEl?.closest("[data-blockid]")?.querySelector<HTMLInputElement>("input[data-pane-focus]");
        // An IME composition starting (CJK and others): move the caret to the
        // filter and let the composition happen there.
        if (e.isComposing || e.key === "Process" || e.keyCode === 229) {
            const input = filter();
            input?.focus({ preventScroll: true });
            return input != null;
        }
        if (e.key.length !== 1 || e.key === " " || e.ctrlKey || e.metaKey || e.altKey) return false;
        const input = filter();
        if (!input) return false;
        e.preventDefault();
        input.focus({ preventScroll: true });
        const end = input.value.length;
        input.setRangeText(e.key, end, end, "end");
        input.dispatchEvent(new Event("input", { bubbles: true }));
        return true;
    };

    const handleNewClick = (e: MouseEvent) => {
        // Don't bubble up — outer card click would auto-continue and
        // race the archive RPC we're about to fire.
        e.stopPropagation();
        if (props.disabled) return;
        props.onNewSession?.(props.agent);
    };

    const handleContextMenu = (e: MouseEvent) => {
        // Only intercept if a handler is wired. Otherwise let the
        // browser show its default menu (useful for "Inspect" in
        // dev). The handler is responsible for preventDefault.
        if (props.onContextMenu) {
            props.onContextMenu(props.agent, e);
        }
    };

    return (
        <div
            ref={cardEl}
            class={`agent-card${props.launching ? " agent-card--launching" : ""}`}
            classList={{
                "agent-card--disabled": props.disabled,
                "agent-card--needs-install": props.installed === false,
                "agent-card--open": !!props.expanded,
                "agent-card--has-tags": chipCount() > 0,
                "agent-card--has-action": hasAction(),
                "agent-card--narrow": narrow(),
            }}
            onClick={handleCardClick}
            onContextMenu={handleContextMenu}
            onKeyDown={handleKeyDown}
            role="button"
            tabIndex={props.disabled ? -1 : 0}
            aria-disabled={props.disabled}
            aria-expanded={props.expanded === undefined ? undefined : props.expanded}
            aria-label={`Launch ${title()}`}
        >
            {/* Harness icon only — no vendor badge. This section is
                explicitly about picking a harness (see the section hint
                text in AgentPicker.tsx: "you'll pick which model it uses
                next"), so the vendor badge DualProviderLogo overlays is
                redundant here. MyAgentsList.tsx (an already-launched
                agent, where the model vendor is a real, relevant fact
                about that specific instance) still uses DualProviderLogo
                with its badge, unchanged. */}
            <ProviderLogo provider={props.agent.provider} size={28} class="agent-card-icon" />
            <span class="agent-card-info">
                <span class="agent-card-title">{title()}</span>
                <span class="agent-card-caption">{caption()}</span>
            </span>
            {/* No runtime badge here. A template is runtime-agnostic —
                the host/container choice is made when you instantiate it
                (in the panel that opens under the card), not a property of the
                template itself. The badge belongs on a launched session
                (MyAgentsList), where the runtime is concrete. */}
            {/* The ribbon (what a click will do) and the chips (Free, Local, an update
                note), as the kit's Tag. The old install ribbon and upgrade badge were
                hand-rolled copies of it. */}
            <TagList tags={tags()} max={narrow() ? 2 : 3} size="xs" class="agent-card-tags" />
            {/* Option E (PR #1008): "+ New" affordance — visible only
                when the agent has an in-progress session. This
                button asks the parent to archive the current zone
                and start fresh. Hidden during install ribbon
                state to avoid double-CTA overlap. */}
            <Show when={props.hasCurrentSession && props.installed !== false && !props.launching}>
                <button
                    type="button"
                    class="agent-card-new-session-btn"
                    onClick={handleNewClick}
                    title="Archive current session and start a new one"
                    aria-label={`Start a new session for ${title()} (archives the current one)`}
                    tabIndex={props.disabled ? -1 : 0}
                >
                    {"+ New"}
                </button>
            </Show>
            <Show when={props.launching}>
                <span class="agent-card-spinner" />
            </Show>
        </div>
    );
};

AgentCard.displayName = "AgentCard";
