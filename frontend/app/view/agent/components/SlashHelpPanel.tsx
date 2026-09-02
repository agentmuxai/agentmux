// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SlashHelpPanel — overlay listing every slash command currently
 * available in the pane, grouped by category.
 *
 * Step 4 of docs/specs/SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md.
 *
 * Reads the command list from props (computed by useAgentCommands via
 * registry.list(ctx)) so the panel automatically reflects new
 * registrations without any wiring here. Click a row to invoke that
 * command — for arg-less ones it runs immediately; for ones with a
 * required enum it triggers the Step 2 picker.
 */

import { type JSX, createMemo, onCleanup, onMount } from "solid-js";
import { For } from "solid-js/web";
import type { SlashCommand, SlashCommandCategory } from "../commands/types";

interface SlashHelpPanelProps {
    commands: SlashCommand[];
    onInvoke: (cmd: SlashCommand) => void;
    onClose: () => void;
}

const CATEGORY_ORDER: SlashCommandCategory[] = [
    "runtime",
    "session",
    "auth",
    "query",
    "system",
    "help",
];

const CATEGORY_LABEL: Record<SlashCommandCategory, string> = {
    runtime: "Runtime",
    session: "Session",
    auth: "Auth",
    query: "Query",
    system: "System",
    help: "Help",
};

function argHint(cmd: SlashCommand): string {
    if (cmd.arg.kind === "none") return "";
    if (cmd.arg.kind === "enum") {
        const choices = typeof cmd.arg.choices === "function" ? "<choice>" : cmd.arg.choices.map((c) => c.value).join(" | ");
        return cmd.arg.required ? `<${choices}>` : `[${choices}]`;
    }
    if (cmd.arg.kind === "freeform") {
        return cmd.arg.required ? `<${cmd.arg.placeholder}>` : `[${cmd.arg.placeholder}]`;
    }
    return `<${cmd.arg.placeholder}>`;
}

export function SlashHelpPanel(props: SlashHelpPanelProps): JSX.Element {
    const grouped = createMemo(() => {
        const byCategory = new Map<SlashCommandCategory, SlashCommand[]>();
        for (const cmd of props.commands) {
            const list = byCategory.get(cmd.category) ?? [];
            list.push(cmd);
            byCategory.set(cmd.category, list);
        }
        return CATEGORY_ORDER.filter((cat) => byCategory.has(cat)).map((cat) => ({
            category: cat,
            label: CATEGORY_LABEL[cat],
            commands: (byCategory.get(cat) ?? []).sort((a, b) => a.name.localeCompare(b.name)),
        }));
    });

    const handleKeyDown = (e: KeyboardEvent): void => {
        if (e.key === "Escape") {
            e.preventDefault();
            props.onClose();
        }
    };

    onMount(() => document.addEventListener("keydown", handleKeyDown, true));
    onCleanup(() => document.removeEventListener("keydown", handleKeyDown, true));

    return (
        <div class="slash-help" role="dialog" aria-label="Slash command help">
            <div class="slash-help__header">
                <span class="slash-help__title">Slash commands</span>
                <button
                    type="button"
                    class="slash-help__close"
                    aria-label="Close help"
                    onClick={() => props.onClose()}
                >
                    ×
                </button>
            </div>
            <div class="slash-help__body">
                <For each={grouped()}>
                    {(group) => (
                        <div class="slash-help__group">
                            <div class="slash-help__group-label">{group.label}</div>
                            <For each={group.commands}>
                                {(cmd) => {
                                    const activate = () => props.onInvoke(cmd);
                                    const onKeyDown = (e: KeyboardEvent) => {
                                        if (e.key === "Enter" || e.key === " ") {
                                            e.preventDefault();
                                            activate();
                                        }
                                    };
                                    const aliasText = (cmd.aliases ?? []).map((a) => `/${a}`).join(" · ");
                                    return (
                                        <div
                                            class="slash-help__row"
                                            role="button"
                                            tabIndex={0}
                                            onClick={activate}
                                            onKeyDown={onKeyDown}
                                        >
                                            <span class="slash-help__name">/{cmd.name}</span>
                                            <span class="slash-help__aliases">{aliasText}</span>
                                            <span class="slash-help__arg">{argHint(cmd)}</span>
                                            <span class="slash-help__description">{cmd.description}</span>
                                        </div>
                                    );
                                }}
                            </For>
                        </div>
                    )}
                </For>
            </div>
            <div class="slash-help__hint">
                <span>Click to run · Esc to close</span>
            </div>
        </div>
    );
}
