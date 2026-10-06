// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { createEffect, createSignal, For, type JSX, onCleanup, Show, splitProps } from "solid-js";
import { useField } from "./Field";
import { densityClass, type UiDensity } from "./shared";

import "./ui.scss";

/** The id / describedby / invalid attributes a control takes from its Field. */
function fieldAttrs(explicitId: () => string | undefined, invalid: () => boolean | undefined = () => false) {
    const field = useField();
    return {
        id: () => explicitId() ?? field?.id,
        describedBy: () => field?.describedBy(),
        invalid: (): "true" | undefined => (invalid() || field?.invalid() ? "true" : undefined),
    };
}

// ── TextInput ─────────────────────────────────────────────────────────────────

export interface TextInputProps extends Omit<JSX.InputHTMLAttributes<HTMLInputElement>, "class" | "classList"> {
    density?: UiDensity;
    invalid?: boolean;
    /**
     * Don't take the enclosing Field's id and description: for one of
     * several inputs in a field (a key/value row), which would otherwise
     * share an id. Name it with `aria-label` instead.
     */
    detached?: boolean;
    class?: string;
}

export function TextInput(props: TextInputProps): JSX.Element {
    const [local, rest] = splitProps(props, ["density", "invalid", "detached", "class", "id", "type"]);
    const field = fieldAttrs(() => local.id, () => local.invalid);
    return (
        <input
            {...rest}
            type={local.type ?? "text"}
            id={local.detached ? local.id : field.id()}
            class={clsx("ui-input", densityClass(local.density), local.class)}
            aria-describedby={local.detached ? undefined : field.describedBy()}
            aria-invalid={field.invalid()}
        />
    );
}

// ── NumberInput ───────────────────────────────────────────────────────────────

export interface NumberInputProps {
    value: number;
    onChange: (value: number) => void;
    min: number;
    max?: number;
    step: number;
    /** Must match what the setting stores. Default "float". */
    parse?: "int" | "float";
    /** Default 400ms. */
    debounceMs?: number;
    density?: UiDensity;
    disabled?: boolean;
    id?: string;
    class?: string;
}

/**
 * A number field that commits while you type, after a pause, and flushes on
 * blur. Out-of-range or unparsable values are dropped. This is
 * settings-controls.tsx's `NumberControl`, whose doc comment records why
 * 400ms and why blur-only commits were a bug
 * (SPEC_SETTINGS_LIVE_COMMIT_AND_TERMINAL_APPLY_GAPS_2026_09_22.md).
 */
export function NumberInput(props: NumberInputProps): JSX.Element {
    const field = fieldAttrs(() => props.id);
    const [local, setLocal] = createSignal(props.value);
    createEffect(() => setLocal(props.value));
    let timer: ReturnType<typeof setTimeout> | null = null;
    onCleanup(() => {
        if (timer != null) clearTimeout(timer);
    });
    const parseVal = (raw: string) => (props.parse === "int" ? parseInt(raw, 10) : parseFloat(raw));
    const inRange = (v: number) => !isNaN(v) && v >= props.min && (props.max == null || v <= props.max);

    return (
        <input
            type="number"
            id={field.id()}
            class={clsx("ui-input", densityClass(props.density), props.class)}
            aria-describedby={field.describedBy()}
            aria-invalid={field.invalid()}
            min={props.min}
            max={props.max}
            step={props.step}
            disabled={props.disabled}
            value={local()}
            onInput={(e) => {
                const v = parseVal(e.currentTarget.value);
                if (!isNaN(v)) setLocal(v);
                if (timer != null) clearTimeout(timer);
                timer = setTimeout(() => {
                    timer = null;
                    if (inRange(v)) props.onChange(v);
                }, props.debounceMs ?? 400);
            }}
            onBlur={(e) => {
                // Flush only a pending commit; one that already fired must not
                // fire a second time for the same value.
                if (timer == null) return;
                clearTimeout(timer);
                timer = null;
                const v = parseVal(e.currentTarget.value);
                if (inRange(v)) props.onChange(v);
            }}
        />
    );
}

// ── Select ────────────────────────────────────────────────────────────────────

export interface SelectOption {
    value: string;
    label: string;
    disabled?: boolean;
}

export interface SelectProps {
    /** The choices. Or pass `<option>` elements as children instead. */
    options?: SelectOption[];
    children?: JSX.Element;
    value: string;
    onChange: (value: string) => void;
    density?: UiDensity;
    disabled?: boolean;
    id?: string;
    ariaLabel?: string;
    class?: string;
}

/**
 * A native select with the line look when closed. The open list is drawn by
 * the OS; `color-scheme` in theme.scss keeps it on the right palette.
 */
export function Select(props: SelectProps): JSX.Element {
    const field = fieldAttrs(() => props.id);
    return (
        <select
            id={field.id()}
            class={clsx("ui-input", densityClass(props.density), props.class)}
            aria-label={props.ariaLabel}
            aria-describedby={field.describedBy()}
            aria-invalid={field.invalid()}
            disabled={props.disabled}
            value={props.value}
            onChange={(e) => props.onChange(e.currentTarget.value)}
        >
            <Show when={props.options} fallback={props.children}>
                {(options) => (
                    <For each={options()}>
                        {(option) => (
                            <option value={option.value} disabled={option.disabled} selected={option.value === props.value}>
                                {option.label}
                            </option>
                        )}
                    </For>
                )}
            </Show>
        </select>
    );
}

// ── Switch ────────────────────────────────────────────────────────────────────

export interface SwitchProps {
    checked: boolean;
    onChange: (checked: boolean) => void;
    density?: UiDensity;
    disabled?: boolean;
    id?: string;
    /** Needed unless the switch sits in a `Field`, whose label names it. */
    ariaLabel?: string;
    class?: string;
}

/** An on/off switch drawn as an outline: on is an accent line and thumb, never a solid track. */
export function Switch(props: SwitchProps): JSX.Element {
    const field = fieldAttrs(() => props.id);
    return (
        <button
            type="button"
            role="switch"
            id={field.id()}
            class={clsx("ui-switch", densityClass(props.density), props.class)}
            aria-checked={props.checked ? "true" : "false"}
            aria-label={props.ariaLabel}
            aria-describedby={field.describedBy()}
            disabled={props.disabled}
            onClick={() => props.onChange(!props.checked)}
        >
            <span class="ui-switch-thumb" />
        </button>
    );
}
