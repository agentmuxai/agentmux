// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The stand-in child for agent-view.wiring.test.tsx: draws its name and the props it was given, and keeps them for the test. */

import type { JSX } from "solid-js";

/** The props each stand-in last received, by component name. */
export const seen: Record<string, any> = {};

export const stub = (name: string, withChildren = false) => (props: Record<string, unknown> & { children?: JSX.Element }) => {
    seen[name] = props;
    const shown: Record<string, string> = {};
    for (const k of Object.keys(props).sort()) {
        if (k === "children") continue;
        let v: unknown;
        try {
            v = props[k];
        } catch {
            v = "<throws>";
        }
        shown[k] = typeof v === "function" ? "<fn>" : v && typeof v === "object" ? "<object>" : (JSON.stringify(v) ?? "undefined");
    }
    return (
        <div data-stub={name} data-props={JSON.stringify(shown)}>
            {withChildren ? props.children : null}
        </div>
    );
};
