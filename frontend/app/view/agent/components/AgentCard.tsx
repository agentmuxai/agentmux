// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCard — a single entry in the agent picker list.
 *
 * PR 2 of specs/SPEC_CONSOLIDATE_FORGE_IDENTITY_INTO_AGENT_2026_04_13.md
 * adds the ⚙ Forge and 👤 Identity buttons that expand an inline
 * settings panel below the card.
 *
 * Step 3 of SPEC_AGENT_IDENTITY_RESTRUCTURE_2026_04_14.md adds:
 * - Slug displayed as small secondary line under the name.
 * - ✏ hover button for inline rename; Enter saves, Esc cancels.
 *
 * Clicking the card body still launches the agent. The buttons call
 * stopPropagation so they never accidentally trigger launch.
 *
 * The card is rendered as a <div role="button"> rather than <button>
 * so nested action buttons are valid HTML (buttons can't contain
 * buttons).
 */

import { createSignal, Show, type JSX } from "solid-js";

interface AgentCardProps {
    agent: ForgeAgent;
    launching: boolean;
    disabled: boolean;
    onLaunch: (agent: ForgeAgent) => void;
    onOpenForge: (agent: ForgeAgent) => void;
    onOpenIdentity: (agent: ForgeAgent) => void;
    /** Called when the user commits a rename via the inline ✏ control. */
    onRename: (agent: ForgeAgent, newName: string) => Promise<string | null>;
}

export const AgentCard = (props: AgentCardProps): JSX.Element => {
    const [renaming, setRenaming] = createSignal(false);
    const [editName, setEditName] = createSignal("");
    const [renameError, setRenameError] = createSignal<string | null>(null);
    const [saving, setSaving] = createSignal(false);

    const stopAndRun = (fn: () => void) => (e: MouseEvent) => {
        e.stopPropagation();
        fn();
    };

    const handleCardClick = () => {
        if (!props.disabled && !renaming()) props.onLaunch(props.agent);
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (props.disabled || renaming()) return;
        if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            props.onLaunch(props.agent);
        }
    };

    const startRename = (e: MouseEvent) => {
        e.stopPropagation();
        setEditName(props.agent.name);
        setRenameError(null);
        setRenaming(true);
    };

    const cancelRename = (e?: MouseEvent) => {
        e?.stopPropagation();
        setRenaming(false);
        setRenameError(null);
    };

    const commitRename = async (e?: MouseEvent) => {
        e?.stopPropagation();
        const newName = editName().trim();
        if (!newName) {
            setRenameError("Name cannot be empty");
            return;
        }
        if (newName === props.agent.name) {
            setRenaming(false);
            return;
        }
        setSaving(true);
        const err = await props.onRename(props.agent, newName);
        setSaving(false);
        if (err) {
            setRenameError(err);
        } else {
            setRenaming(false);
            setRenameError(null);
        }
    };

    const handleInputKeyDown = (e: KeyboardEvent) => {
        e.stopPropagation();
        if (e.key === "Enter") {
            e.preventDefault();
            void commitRename();
        } else if (e.key === "Escape") {
            e.preventDefault();
            cancelRename();
        }
    };

    const slug = () => props.agent.slug || "";

    return (
        <div
            class={`agent-card${props.launching ? " agent-card--launching" : ""}`}
            classList={{ "agent-card--disabled": props.disabled }}
            onClick={handleCardClick}
            onKeyDown={handleKeyDown}
            role="button"
            tabIndex={props.disabled ? -1 : 0}
            aria-disabled={props.disabled}
        >
            <span class="agent-card-icon">{props.agent.icon}</span>
            <span class="agent-card-info">
                <Show
                    when={renaming()}
                    fallback={
                        <>
                            <span class="agent-card-name">{props.agent.name}</span>
                            <Show when={slug()}>
                                <span class="agent-card-slug">{slug()}</span>
                            </Show>
                            <Show when={props.agent.description}>
                                <span class="agent-card-desc">{props.agent.description}</span>
                            </Show>
                        </>
                    }
                >
                    <div class="agent-card-rename-row" onClick={(e) => e.stopPropagation()}>
                        <input
                            class={`agent-card-rename-input${renameError() ? " agent-card-rename-input--error" : ""}`}
                            type="text"
                            value={editName()}
                            onInput={(e) => {
                                setEditName(e.currentTarget.value);
                                setRenameError(null);
                            }}
                            onKeyDown={handleInputKeyDown}
                            disabled={saving()}
                            ref={(el) => setTimeout(() => el?.focus(), 0)}
                            aria-label="Rename agent"
                        />
                        <button
                            class="agent-card-rename-btn agent-card-rename-btn--confirm"
                            onClick={(e) => { e.stopPropagation(); void commitRename(); }}
                            disabled={saving()}
                            type="button"
                            title="Save (Enter)"
                        >
                            {"\u2713"}
                        </button>
                        <button
                            class="agent-card-rename-btn agent-card-rename-btn--cancel"
                            onClick={cancelRename}
                            disabled={saving()}
                            type="button"
                            title="Cancel (Esc)"
                        >
                            {"\u2715"}
                        </button>
                    </div>
                    <Show when={renameError()}>
                        <span class="agent-card-desc" style={{ color: "var(--error-color, #e55)" }}>{renameError()}</span>
                    </Show>
                </Show>
            </span>
            <Show when={!renaming()}>
                <div class="agent-card-actions">
                    <button
                        class="agent-card-action-btn"
                        onClick={startRename}
                        title="Rename this agent"
                        disabled={props.disabled}
                        type="button"
                    >
                        {"\u270F"}
                    </button>
                    <button
                        class="agent-card-action-btn"
                        onClick={stopAndRun(() => props.onOpenForge(props.agent))}
                        title="Configure this agent in the Forge"
                        disabled={props.disabled}
                        type="button"
                    >
                        {"\u2699"}
                    </button>
                    <button
                        class="agent-card-action-btn"
                        onClick={stopAndRun(() => props.onOpenIdentity(props.agent))}
                        title="Manage this agent's identity"
                        disabled={props.disabled}
                        type="button"
                    >
                        {"\uD83D\uDC64"}
                    </button>
                </div>
            </Show>
            <Show when={props.launching}>
                <span class="agent-card-spinner" />
            </Show>
        </div>
    );
};

AgentCard.displayName = "AgentCard";
