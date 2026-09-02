// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SlashCommandPicker — inline picker shown above the composer when the
 * user submits a bare `/cmd` whose arg is a required enum.
 *
 * Step 2 of docs/specs/SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md.
 *
 * Lifecycle is owned by useAgentCommands: the dispatcher calls
 * `ctx.openPicker(spec)`, which sets the hook's pickerSpec signal and
 * returns a Promise. This component renders when the signal is non-null
 * and resolves/rejects the Promise via the `onSelect` / `onDismiss` props.
 */

import { type JSX, createSignal, onCleanup, onMount } from "solid-js";
import { For } from "solid-js/web";
import type { SlashChoice, SlashPickerSpec } from "../commands/types";

interface SlashCommandPickerProps {
    spec: SlashPickerSpec;
    onSelect: (value: string) => void;
    onDismiss: () => void;
}

export function SlashCommandPicker(props: SlashCommandPickerProps): JSX.Element {
    const initialIndex = (): number => {
        const idx = props.spec.choices.findIndex((c) => c.current);
        return idx >= 0 ? idx : 0;
    };
    const [selectedIndex, setSelectedIndex] = createSignal(initialIndex());

    let containerRef: HTMLDivElement | undefined;

    const move = (delta: number): void => {
        const choices = props.spec.choices;
        if (choices.length === 0) return;
        const next = (selectedIndex() + delta + choices.length) % choices.length;
        setSelectedIndex(next);
    };

    const commit = (): void => {
        const choice = props.spec.choices[selectedIndex()];
        if (choice) props.onSelect(choice.value);
    };

    const handleKeyDown = (e: KeyboardEvent): void => {
        if (e.key === "ArrowDown") {
            e.preventDefault();
            move(1);
            return;
        }
        if (e.key === "ArrowUp") {
            e.preventDefault();
            move(-1);
            return;
        }
        if (e.key === "Enter") {
            e.preventDefault();
            commit();
            return;
        }
        if (e.key === "Escape") {
            e.preventDefault();
            props.onDismiss();
            return;
        }
        // Letter-jump: first choice whose label or value starts with the key.
        if (e.key.length === 1 && /[a-z0-9]/i.test(e.key)) {
            const k = e.key.toLowerCase();
            const idx = props.spec.choices.findIndex(
                (c) => c.label.toLowerCase().startsWith(k) || c.value.toLowerCase().startsWith(k),
            );
            if (idx >= 0) {
                e.preventDefault();
                setSelectedIndex(idx);
            }
        }
    };

    onMount(() => {
        document.addEventListener("keydown", handleKeyDown, true);
        // Scroll into view in case the composer region is short.
        requestAnimationFrame(() => containerRef?.scrollIntoView({ block: "nearest" }));
    });
    onCleanup(() => {
        document.removeEventListener("keydown", handleKeyDown, true);
    });

    const handleRowClick = (idx: number) => (e: MouseEvent) => {
        e.preventDefault();
        setSelectedIndex(idx);
        commit();
    };

    return (
        <div class="slash-picker" ref={containerRef} role="listbox" aria-label={props.spec.title}>
            <div class="slash-picker__title">{props.spec.title}</div>
            <div class="slash-picker__list">
                <For each={props.spec.choices}>
                    {(choice: SlashChoice, idx) => (
                        <div
                            class="slash-picker__row"
                            classList={{
                                "slash-picker__row--selected": idx() === selectedIndex(),
                                "slash-picker__row--current": !!choice.current,
                            }}
                            role="option"
                            aria-selected={idx() === selectedIndex()}
                            onMouseEnter={() => setSelectedIndex(idx())}
                            onClick={handleRowClick(idx())}
                        >
                            <span class="slash-picker__marker">{choice.current ? "●" : ""}</span>
                            <span class="slash-picker__label">{choice.label}</span>
                            {choice.description && (
                                <span class="slash-picker__description">{choice.description}</span>
                            )}
                        </div>
                    )}
                </For>
            </div>
            <div class="slash-picker__hint">
                <span>↑↓ navigate</span>
                <span>↵ select</span>
                <span>Esc cancel</span>
            </div>
        </div>
    );
}
