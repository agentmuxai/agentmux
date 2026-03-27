// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { JSX, Show } from "solid-js";
import "./toggle.scss";

interface ToggleProps {
    checked: boolean;
    onChange: (value: boolean) => void;
    label?: string;
    id?: string;
}

const Toggle = ({ checked, onChange, label, id }: ToggleProps): JSX.Element => {
    let inputRef!: HTMLInputElement;

    const handleChange = (e: Event) => {
        if (onChange != null) {
            onChange((e.target as HTMLInputElement).checked);
        }
    };

    const handleLabelClick = () => {
        if (inputRef) {
            inputRef.click();
        }
    };

    const inputId = id || `toggle-${Math.random().toString(36).substr(2, 9)}`;

    return (
        <div class="check-toggle-wrapper">
            <label for={inputId} class="checkbox-toggle">
                <input id={inputId} type="checkbox" checked={checked} onChange={handleChange} ref={inputRef} />
                <span class="slider" />
            </label>
            <Show when={label}>
                <span class="toggle-label" onClick={handleLabelClick}>
                    {label}
                </span>
            </Show>
        </div>
    );
};

export { Toggle };
