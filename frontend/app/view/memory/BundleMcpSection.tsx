// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BundleMcpSection — the "MCP Servers" section of the Bundle editor's
 * detail view (memory-manager.tsx). Composable model v2,
 * docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md, GH issue #2024
 * item 3.
 *
 * Two distinct actions, both surfaced here:
 *   - Bind/unbind an EXISTING global server. Offered for parity with the
 *     agent-scoped AgentMcpModal and because it's harmless, but has NO
 *     effect on a spawned agent — a global server already reaches every
 *     agent unconditionally regardless of bundle binding (see
 *     BundleMcpModel's doc comment). The caveat text below says so.
 *   - Add a brand-new PRIVATE server scoped to this bundle. This is the
 *     actually-functional path: every agent bound to this bundle gets it.
 *
 * Deliberately a flat inline list, not a nested PrimitiveListDetail — this
 * section already lives inside the bundle's own detail pane (itself one
 * side of MemoryManagerBody's list/detail split), and a second full-height
 * single-pane swap nested inside that would fight for the same space.
 */

import { createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { BundleMcpModel } from "./bundle-mcp-model";
import "./BundlePrimitiveSection.scss";

interface BundleMcpSectionProps {
    bundleId: string;
}

export const BundleMcpSection = (props: BundleMcpSectionProps): JSX.Element => {
    const model = new BundleMcpModel(props.bundleId);
    onCleanup(() => model.dispose());

    const [newName, setNewName] = createSignal("");
    const [newConfig, setNewConfig] = createSignal("{}");

    const handleAdd = (e: Event) => {
        e.preventDefault();
        const name = newName().trim();
        if (!name) return;
        void model.addPrivate(name, newConfig()).then(() => {
            setNewName("");
            setNewConfig("{}");
        });
    };

    return (
        <div class="bundle-primitive-section">
            <h4 class="bundle-primitive-section-title">MCP Servers</h4>
            <Show when={model.errorAtom()}>
                <div class="bundle-primitive-section-error">{model.errorAtom()}</div>
            </Show>
            <Show
                when={model.serversAtom().length > 0}
                fallback={<p class="bundle-primitive-section-empty">No MCP servers yet</p>}
            >
                <ul class="bundle-primitive-section-list">
                    <For each={model.serversAtom()}>
                        {(server) => (
                            <li class="bundle-primitive-section-item">
                                <span class="bundle-primitive-section-item-name">{server.name}</span>
                                <Show when={!server.is_global}>
                                    <span class="bundle-primitive-section-item-badge is-private">private</span>
                                </Show>
                                <Show
                                    when={server.bound_to_bundle}
                                    fallback={
                                        <Show when={server.is_global}>
                                            <button
                                                type="button"
                                                class="bundle-primitive-section-btn"
                                                onClick={() => void model.bind(server.id)}
                                            >
                                                Bind
                                            </button>
                                        </Show>
                                    }
                                >
                                    <button
                                        type="button"
                                        class="bundle-primitive-section-btn"
                                        onClick={() => void model.unbind(server.id)}
                                    >
                                        {server.is_global ? "Unbind" : "Remove"}
                                    </button>
                                </Show>
                            </li>
                        )}
                    </For>
                </ul>
            </Show>
            <p class="bundle-primitive-section-caveat">
                Binding an existing global server here has no effect on a spawned agent
                — it's already applied to every agent regardless of bundle binding. Add a
                new private server below to give this bundle its own tool.
            </p>
            <form class="bundle-primitive-section-add" onSubmit={handleAdd}>
                <input
                    type="text"
                    class="bundle-primitive-section-add-input"
                    placeholder="Server name"
                    value={newName()}
                    onInput={(e) => setNewName(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                />
                <textarea
                    class="bundle-primitive-section-add-textarea"
                    placeholder='{"command": "my-tool"}'
                    rows={3}
                    value={newConfig()}
                    onInput={(e) => setNewConfig(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                />
                <button
                    type="submit"
                    class="bundle-primitive-section-add-btn"
                    disabled={model.addingAtom() || !newName().trim()}
                >
                    {model.addingAtom() ? "Adding…" : "+ Add private server"}
                </button>
            </form>
        </div>
    );
};

BundleMcpSection.displayName = "BundleMcpSection";
