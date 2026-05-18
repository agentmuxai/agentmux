// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCard — a definition tile in the agent picker.
 *
 * Per SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23 the card is driven
 * by the CLI catalog when one exists for the provider — title reads
 * as a capability blurb ("Anthropic's coding agent") and the CLI
 * brand name ("Claude Code") sits as a caption below. Providers
 * without a catalog entry fall back to the ForgeAgent row's own
 * description/name fields.
 *
 * Clicking the body opens the Launch modal (or Install modal if
 * the CLI isn't installed yet). When `installed === false`, a
 * bottom-right "Click to install" ribbon is overlaid on the card —
 * like a "For Sale" sign. The ribbon disappears once installation
 * succeeds (parent re-runs install.check after the install modal
 * closes).
 */

import { createMemo, Show, type JSX } from "solid-js";
import { ProviderLogo } from "@/element/ProviderLogo";
import { getCliCatalogEntry } from "../defaults/cli-catalog";

interface AgentCardProps {
    agent: ForgeAgent;
    launching: boolean;
    disabled: boolean;
    /** undefined = not yet checked / non-npm provider (no install needed).
     *  true = CLI present in the per-version cache.
     *  false = needs install — render the bottom-right ribbon. */
    installed: boolean | undefined;
    /** Opens the AgentLaunchModal (or Install modal) for this definition. */
    onLaunch: (agent: ForgeAgent) => void;
}

export const AgentCard = (props: AgentCardProps): JSX.Element => {
    const catalog = createMemo(() => getCliCatalogEntry(props.agent.provider));
    const title = () => catalog()?.blurb || props.agent.description || props.agent.name;
    const caption = () => catalog()?.displayName || props.agent.name;

    const handleCardClick = () => {
        if (!props.disabled) props.onLaunch(props.agent);
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (props.disabled) return;
        if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            props.onLaunch(props.agent);
        }
    };

    return (
        <div
            class={`agent-card${props.launching ? " agent-card--launching" : ""}`}
            classList={{
                "agent-card--disabled": props.disabled,
                "agent-card--needs-install": props.installed === false,
            }}
            onClick={handleCardClick}
            onKeyDown={handleKeyDown}
            role="button"
            tabIndex={props.disabled ? -1 : 0}
            aria-disabled={props.disabled}
            aria-label={`Launch ${caption()}`}
        >
            <ProviderLogo provider={props.agent.provider} size={28} class="agent-card-icon" />
            <span class="agent-card-info">
                <span class="agent-card-title">{title()}</span>
                <span class="agent-card-caption">{caption()}</span>
            </span>
            <Show when={props.installed === false}>
                <span class="agent-card-install-ribbon" aria-hidden="true">
                    Click to install
                </span>
            </Show>
            <Show when={props.launching}>
                <span class="agent-card-spinner" />
            </Show>
        </div>
    );
};

AgentCard.displayName = "AgentCard";
