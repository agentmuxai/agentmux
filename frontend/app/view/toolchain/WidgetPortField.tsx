// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, Show, type JSX } from "solid-js";

import { Field, IconButton, TextInput } from "@/app/element/ui";
import { parsePort } from "./toolchain-status";

export interface WidgetPortFieldProps {
    /** The port in use: the saved one, or the default. */
    port: number;
    defaultPort: number;
    /** A port other than the default is saved. */
    custom: boolean;
    /** A valid port, or undefined to go back to the default. */
    onApply: (port: number | undefined) => void;
}

/**
 * A widget's port. It is applied on Enter or when the field loses focus, and
 * a value that isn't a port says so under the field, with the port still in use.
 */
export function WidgetPortField(props: WidgetPortFieldProps): JSX.Element {
    const [draft, setDraft] = createSignal(String(props.port));
    const [error, setError] = createSignal("");
    // The port in use changed (a reset, or the saved one loading): show it.
    createEffect(() => {
        setDraft(String(props.port));
        setError("");
    });

    const commit = () => {
        const port = parsePort(draft());
        if (port === null) {
            setError(`Enter a whole number from 1 to 65535. Still using ${props.port}.`);
            return;
        }
        setError("");
        if (port !== props.port) props.onApply(port);
    };

    return (
        <Field label="Port" hint={`${props.defaultPort} unless you change it.`} error={error() || undefined}>
            <div class="toolchain-detail-row">
                <TextInput
                    class="toolchain-port-input"
                    density="compact"
                    inputMode="numeric"
                    value={draft()}
                    onInput={(e) => setDraft(e.currentTarget.value)}
                    onBlur={commit}
                    onKeyDown={(e) => {
                        if (e.key === "Enter") commit();
                    }}
                />
                <Show when={props.custom}>
                    <IconButton
                        density="compact"
                        icon="rotate-left"
                        label="Reset to the default port"
                        onClick={() => props.onApply(undefined)}
                    />
                </Show>
            </div>
        </Field>
    );
}
