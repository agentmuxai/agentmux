// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The stand-in child for AgentBottomPanels.behavior.test.tsx: draws its name and the props it was given (the ones
 * named in `read` are accessors, read where they are drawn so a change shows), and has a button for each callback
 * named in `call` that presses it with the arguments given and logs the call and what it returned.
 */

import { For, type JSX } from "solid-js";

/** The ordered log of the test: every press of a stand-in's callback, and every call the panels make. */
export const calls: string[] = [];

interface Spec {
    read?: string[];
    call?: Record<string, unknown[]>;
    children?: boolean;
}

const show = (v: unknown): string => {
    if (typeof v === "function") return "<fn>";
    if (v !== null && typeof v === "object") {
        try {
            return JSON.stringify(v);
        } catch {
            return "<object>";
        }
    }
    return JSON.stringify(v) ?? "undefined";
};

export const stand = (name: string, spec: Spec = {}) => (props: Record<string, any> & { children?: JSX.Element }) => {
    const shown = () => {
        const out: Record<string, string> = {};
        for (const k of Object.keys(props).sort()) {
            if (k === "children") continue;
            let v: unknown;
            try {
                v = spec.read?.includes(k) ? props[k]() : props[k];
            } catch {
                v = "<throws>";
            }
            out[k] = show(v);
        }
        return JSON.stringify(out);
    };
    const press = (k: string) => {
        const args = spec.call![k];
        calls.push(`${name}.${k}(${JSON.stringify(args)})`);
        const result = props[k](...args);
        if (result !== undefined) calls.push(`  -> ${show(result)}`);
    };
    return (
        <div data-stand={name} data-props={shown()}>
            <For each={Object.keys(spec.call ?? {})}>
                {(k) => (
                    <span data-cb={k} onClick={() => press(k)}>
                        {k}
                    </span>
                )}
            </For>
            {spec.children ? props.children : null}
        </div>
    );
};
