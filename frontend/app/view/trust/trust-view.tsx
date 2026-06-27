// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, For, type JSX } from "solid-js";

import { IdentityManager } from "@/app/view/identity/identity-manager";
import { MemoryManager } from "@/app/view/memory/memory-manager";
import { AccountsManager } from "@/app/view/accounts/accounts-manager";
import { GlobalBrainManager } from "@/app/view/brain/global-brain-manager";
import type { TrustSection, TrustViewModel } from "./trust-model";
import "./trust-view.scss";

const RAIL: { id: TrustSection; label: string; icon: string }[] = [
    { id: "accounts",   label: "Accounts",   icon: "key" },
    { id: "identities", label: "Identities", icon: "id-card" },
    { id: "brain",      label: "Brain",      icon: "brain" },
    { id: "memories",   label: "Presets",    icon: "sliders" },
];

export function TrustView(_props: ViewComponentProps<TrustViewModel>): JSX.Element {
    const [section, setSection] = createSignal<TrustSection>("accounts");

    return (
        <div class="trust-view">
            <nav class="bundle-manager-rail" aria-label="Trust Center section">
                <For each={RAIL}>
                    {(item) => (
                        <button
                            type="button"
                            class="bundle-manager-rail-item"
                            classList={{ "is-active": section() === item.id }}
                            aria-pressed={section() === item.id}
                            onClick={() => setSection(item.id)}
                        >
                            <i class={`fa-sharp fa-solid fa-${item.icon}`} aria-hidden="true" />
                            <span>{item.label}</span>
                        </button>
                    )}
                </For>
            </nav>
            <nav class="bundle-manager-tab-bar" aria-label="Trust Center section">
                <For each={RAIL}>
                    {(item) => (
                        <button
                            type="button"
                            classList={{ "is-active": section() === item.id }}
                            aria-pressed={section() === item.id}
                            onClick={() => setSection(item.id)}
                        >
                            <i class={`fa-sharp fa-solid fa-${item.icon}`} aria-hidden="true" />
                            <span>{item.label}</span>
                        </button>
                    )}
                </For>
            </nav>
            <div class="bundle-manager-section">
                {/*
                 * All four managers stay mounted — toggling is instant and
                 * never re-fetches. Both stay consistent via WPS *:changed events.
                 */}
                <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "accounts" }}>
                    <AccountsManager />
                </div>
                <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "identities" }}>
                    <IdentityManager />
                </div>
                <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "brain" }}>
                    <GlobalBrainManager />
                </div>
                <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "memories" }}>
                    <MemoryManager />
                </div>
            </div>
        </div>
    );
}
