// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCard — a definition tile in the agent picker.
 *
 * Per SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23 the card is driven
 * by the CLI catalog when one exists for the provider — title reads
 * as a capability blurb ("Anthropic's coding agent") and the CLI
 * brand name ("Claude Code") sits as a caption below. Providers
 * without a catalog entry fall back to the AgentDefinition row's own
 * description/name fields.
 *
 * Clicking the body opens the Launch modal (or Install modal if
 * the CLI isn't installed yet). When `installed === false`, a
 * bottom-right "Click to install" ribbon is overlaid on the card —
 * like a "For Sale" sign. The ribbon disappears once installation
 * succeeds (parent re-runs install.check after the install modal
 * closes).
 */

import { createMemo, onMount, Show, type JSX } from "solid-js";
import { ProviderLogo } from "@/element/ProviderLogo";
import { getCliCatalogEntry } from "../defaults/cli-catalog";

interface AgentCardProps {
    agent: AgentDefinition;
    launching: boolean;
    disabled: boolean;
    /** undefined = not yet checked / non-npm provider (no install needed).
     *  true = CLI present in the per-version cache.
     *  false = needs install — render the bottom-right ribbon. */
    installed: boolean | undefined;
    /**
     * Opens the AgentLaunchModal (or Install modal) for this definition.
     * The synthetic `MouseEvent` is forwarded so the parent can read
     * modifier keys (Shift/Ctrl/Alt) — used by Option E to force the
     * launch modal even when the agent has an in-progress session
     * (default click would auto-continue otherwise).
     */
    onLaunch: (agent: AgentDefinition, evt?: MouseEvent | KeyboardEvent) => void;
    /**
     * Option E (PR 2 of 2): when true, this card's agent has a
     * non-empty session zone (`agent:<defId>:current`) — i.e. the
     * default click will auto-continue rather than open the launch
     * modal. Renders a small "+ New" secondary button that archives
     * the current zone and opens the launch modal for a fresh start.
     */
    hasCurrentSession?: boolean;
    /** Option E: invoked when the user clicks the "+ New" affordance.
     *  Parent archives the current zone then opens the launch modal. */
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
}

export const AgentCard = (props: AgentCardProps): JSX.Element => {
    const catalog = createMemo(() => getCliCatalogEntry(props.agent.provider));
    const title = () => catalog()?.blurb || props.agent.description || props.agent.name;
    const caption = () => catalog()?.displayName || props.agent.name;

    let cardEl: HTMLDivElement | undefined;
    onMount(() => {
        if (props.defaultFocus && !props.disabled) cardEl?.focus();
    });

    const handleCardClick = (e: MouseEvent) => {
        if (!props.disabled) props.onLaunch(props.agent, e);
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (props.disabled) return;
        if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            props.onLaunch(props.agent, e);
        }
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
            }}
            onClick={handleCardClick}
            onContextMenu={handleContextMenu}
            onKeyDown={handleKeyDown}
            role="button"
            tabIndex={props.disabled ? -1 : 0}
            aria-disabled={props.disabled}
            aria-label={`Launch ${caption()}`}
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
                (in the create-from-template modal), not a property of the
                template itself. The badge belongs on a launched session
                (MyAgentsList), where the runtime is concrete. */}
            <Show when={props.installed === false}>
                <span class="agent-card-install-ribbon" aria-hidden="true">
                    Click to install
                </span>
            </Show>
            {/* Option E (PR #1008): "+ New" affordance — visible only
                when the agent has an in-progress session. Outer card
                click auto-continues by default; this button archives
                the current zone and opens the launch modal so the
                user can start fresh. Hidden during install ribbon
                state to avoid double-CTA overlap. */}
            <Show when={props.hasCurrentSession && props.installed !== false && !props.launching}>
                <button
                    type="button"
                    class="agent-card-new-session-btn"
                    onClick={handleNewClick}
                    title="Archive current session and start a new one"
                    aria-label={`Start a new session for ${caption()} (archives the current one)`}
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
