// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A sample trusted widget (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md
// §9): a Solid ES module whose default export is a pane tab manifest
// (docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §3).
//
// Install: Settings → Widgets → Install…, and pick this folder's widget.json.
// AgentMux copies it to ~/.agentmux/widgets/agentmux.hello/ and asks you to
// approve it; a trusted widget runs as part of AgentMux, so read it first.
//
// Import Solid from "solid-js" (and "solid-js/web", "solid-js/store") as
// usual: the loader points those imports at the app's own Solid, so the
// widget shares its reactive runtime. It reaches its block only through the
// host context `ctx` — no app internals. Its view name comes from its
// package (`ext:agentmux.hello/main`), so it doesn't declare one.

import { createEffect, createSignal } from "solid-js";

export default {
    apiVersion: 1,
    label: "Hello",
    icon: "hand",
    create(ctx) {
        // Persisted in the block's own meta, so it survives restarts.
        const clicks = () => Number(ctx.meta()?.["hello:clicks"] ?? 0);
        // How many times the user has come back to this tab.
        const [shown, setShown] = createSignal(0);

        return {
            liveTitle: () => ({ text: `Hello (${clicks()})` }),
            onActivate: () => setShown((n) => n + 1),
            component: () => {
                const root = document.createElement("div");
                root.className = "ext-hello";
                root.style.padding = "12px";

                const text = document.createElement("p");
                const button = document.createElement("button");
                button.textContent = "Click me";
                button.addEventListener("click", () => ctx.setMeta({ "hello:clicks": clicks() + 1 }));

                createEffect(() => {
                    text.textContent = `Clicked ${clicks()} times; shown ${shown()} times; ${ctx.visibility()}.`;
                });
                root.append(text, button);
                return root;
            },
        };
    },
};
