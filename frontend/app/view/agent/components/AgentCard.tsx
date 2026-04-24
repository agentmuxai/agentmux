// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCard — a definition tile in the agent picker.
 *
 * After SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23 the card is driven
 * by the CLI catalog, not by the ForgeAgent row's user-facing fields.
 * Title reads as a capability blurb ("Anthropic's coding agent") and
 * the CLI brand name sits as a caption below. Clicking the body opens
 * the Launch modal instead of launching directly.
 *
 * The ⓘ button uses the Popover primitive (PR F of the definitions
 * spec) so users get the catalog's full CLI deep-dive text — startup
 * flow, MCP support, memory behaviour — on click, rather than a
 * native tooltip truncated by the OS.
 *
 * The card is rendered as a <div role="button"> rather than <button>
 * so nested action buttons are valid HTML (buttons can't contain
 * buttons).
 */

import { createMemo, Show, useContext, type Component, type JSX } from "solid-js";
import { Popover, PopoverContext, PopoverContent } from "@/element/popover";
import { getCliCatalogEntry } from "../defaults/cli-catalog";

interface AgentCardProps {
    agent: ForgeAgent;
    launching: boolean;
    disabled: boolean;
    /** Opens the AgentLaunchModal for this definition. */
    onLaunch: (agent: ForgeAgent) => void;
    /** Opens the inline settings panel (Forge tab). */
    onOpenForge: (agent: ForgeAgent) => void;
    /** Called when the user clicks the 🗑 delete button. Caller is
     *  responsible for confirmation + the `DeleteForgeAgent` RPC. */
    onDelete: (agent: ForgeAgent) => void;
}

/**
 * The ⓘ trigger. Custom trigger (not `PopoverButton`) so we keep
 * the existing `agent-card-action-btn` chrome instead of inheriting
 * the default `Button` styling. Uses `PopoverContext` directly.
 */
const InfoPopoverTrigger: Component<{ ariaLabel: string }> = (props) => {
    const ctx = useContext(PopoverContext);
    return (
        <button
            ref={(el) => ctx?.registerReference(el)}
            class="agent-card-action-btn agent-card-action-btn--info"
            classList={{ "agent-card-action-btn--open": ctx?.isOpen() }}
            onClick={(e) => {
                e.stopPropagation();
                ctx?.togglePopover();
            }}
            type="button"
            aria-label={props.ariaLabel}
            aria-expanded={ctx?.isOpen()}
        >
            {"\u24D8"}
        </button>
    );
};

export const AgentCard = (props: AgentCardProps): JSX.Element => {
    const catalog = createMemo(() => getCliCatalogEntry(props.agent.provider));

    const icon = () => props.agent.icon || catalog()?.icon || "•";
    const title = () => catalog()?.blurb || props.agent.description || props.agent.name;
    const caption = () => catalog()?.displayName || props.agent.name;
    const popoverText = () => catalog()?.popoverMarkdown ?? props.agent.description ?? "";
    const primaryContextFile = () => catalog()?.primaryContextFile ?? "";
    const mcpSupport = () => catalog()?.mcpSupport ?? "";

    const stopAndRun = (fn: () => void) => (e: MouseEvent) => {
        e.stopPropagation();
        fn();
    };

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
            classList={{ "agent-card--disabled": props.disabled }}
            onClick={handleCardClick}
            onKeyDown={handleKeyDown}
            role="button"
            tabIndex={props.disabled ? -1 : 0}
            aria-disabled={props.disabled}
            aria-label={`Launch ${caption()}`}
        >
            <span class="agent-card-icon">{icon()}</span>
            <span class="agent-card-info">
                <span class="agent-card-title">{title()}</span>
                <span class="agent-card-caption">{caption()}</span>
            </span>
            <div class="agent-card-actions" onClick={(e) => e.stopPropagation()}>
                <Show when={popoverText()}>
                    <Popover placement="right-start" offset={8}>
                        <InfoPopoverTrigger ariaLabel={`About ${caption()}`} />
                        <PopoverContent className="agent-card-info-popover">
                            <div class="agent-card-info-popover-title">{caption()}</div>
                            <Show when={primaryContextFile()}>
                                <div class="agent-card-info-popover-meta">
                                    <span class="agent-card-info-popover-label">Context file</span>
                                    <code>{primaryContextFile()}</code>
                                </div>
                            </Show>
                            <Show when={mcpSupport() && mcpSupport() !== "none"}>
                                <div class="agent-card-info-popover-meta">
                                    <span class="agent-card-info-popover-label">MCP</span>
                                    <code>{mcpSupport()}</code>
                                </div>
                            </Show>
                            <div class="agent-card-info-popover-body">{popoverText()}</div>
                        </PopoverContent>
                    </Popover>
                </Show>
                <button
                    class="agent-card-action-btn"
                    onClick={stopAndRun(() => props.onOpenForge(props.agent))}
                    title="Configure this definition in the Forge"
                    disabled={props.disabled}
                    type="button"
                >
                    {"\u2699"}
                </button>
                <button
                    class="agent-card-action-btn agent-card-action-btn--delete"
                    onClick={stopAndRun(() => props.onDelete(props.agent))}
                    title="Delete this definition"
                    disabled={props.disabled}
                    type="button"
                >
                    {"\uD83D\uDDD1"}
                </button>
            </div>
            <Show when={props.launching}>
                <span class="agent-card-spinner" />
            </Show>
        </div>
    );
};

AgentCard.displayName = "AgentCard";
