// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A sample third-party pane tab — Pane Tab contract v1, Phase 6
// (docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §3, §4).
//
// Install: copy this directory to ~/.agentmux/widgets/hello/ and add to
// widgets.json:
//
//   "ext@hello": {
//       "label": "Hello",
//       "icon": "hand",
//       "module": "hello/index.js",
//       "blockdef": { "meta": { "view": "ext:hello" } }
//   }
//
// A widget is a plain ES module whose default export is a PaneTabManifest.
// Import Solid from "solid-js" (and "solid-js/web", "solid-js/store") as
// usual: the loader points those imports at the app's own Solid, so the
// widget shares its reactive runtime. It reaches its block only through the
// host context `ctx` — no app internals.

import { createEffect, createSignal } from "solid-js";

export default {
    apiVersion: 1,
    view: "ext:hello",
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
