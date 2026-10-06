// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { createContext, createUniqueId, type JSX, Show, useContext } from "solid-js";

import "./ui.scss";

interface FieldContextValue {
    /** The id the control inside should take, so the label points at it. */
    id: string;
    labelId: string;
    describedBy: () => string | undefined;
    invalid: () => boolean;
}

const FieldContext = createContext<FieldContextValue>();

/** For controls: the enclosing `Field`'s wiring, if there is one. */
export function useField(): FieldContextValue | undefined {
    return useContext(FieldContext);
}

export interface FieldProps {
    label: JSX.Element;
    /** Shown under the label: what the setting does. */
    description?: JSX.Element;
    /** Shown under the control: a short note on the value. */
    hint?: JSX.Element;
    /** Shown under the control in the error colour; marks the control invalid. */
    error?: string;
    /** `inline` (default): text left, control right, stacking when narrow. */
    layout?: "inline" | "stacked";
    /** Id for the control. Generated when omitted. */
    id?: string;
    class?: string;
    children: JSX.Element;
}

/**
 * A labelled row: label and description, plus one control. Controls from
 * element/ui/ inside it pick up its id, `aria-describedby` and invalid
 * state on their own.
 */
export function Field(props: FieldProps): JSX.Element {
    const generated = createUniqueId();
    const id = () => props.id ?? `ui-field-${generated}`;
    const labelId = `ui-field-label-${generated}`;
    const descriptionId = `ui-field-desc-${generated}`;
    const hintId = `ui-field-hint-${generated}`;
    const errorId = `ui-field-error-${generated}`;

    const describedBy = () => {
        const ids = [
            props.description != null && descriptionId,
            props.hint != null && hintId,
            props.error && errorId,
        ].filter(Boolean);
        return ids.length > 0 ? ids.join(" ") : undefined;
    };

    const context: FieldContextValue = {
        get id() {
            return id();
        },
        labelId,
        describedBy,
        invalid: () => !!props.error,
    };

    return (
        <div class={clsx("ui-field", `ui-field--${props.layout ?? "inline"}`, props.class)}>
            <div class="ui-field-text">
                <label class="ui-field-label" id={labelId} for={id()}>
                    {props.label}
                </label>
                <Show when={props.description != null}>
                    <div class="ui-field-description" id={descriptionId}>
                        {props.description}
                    </div>
                </Show>
            </div>
            <div class="ui-field-control">
                <FieldContext.Provider value={context}>{props.children}</FieldContext.Provider>
                <Show when={props.hint != null}>
                    <div class="ui-field-hint" id={hintId}>
                        {props.hint}
                    </div>
                </Show>
                <Show when={props.error}>
                    <div class="ui-field-error" id={errorId}>
                        {props.error}
                    </div>
                </Show>
            </div>
        </div>
    );
}
