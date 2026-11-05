// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The bundle manager's rail: the error banner, New and Import, and the list. */

import { For, Show, type JSX } from "solid-js";
import { Button, Tag } from "@/app/element/ui";
import type { BundleActions } from "./bundle-manager-actions";
import type { BundleViewModel } from "./bundle-model";

export function BundleList(props: { model: BundleViewModel; act: BundleActions }): JSX.Element {
    const { model, act } = props;
    return (
        <div class="bundle-view-rail">
            <Show when={model.errorAtom()}>
                <div class="bundle-view-error">{model.errorAtom()}</div>
            </Show>
            <div class="bundle-view-rail-header">
                <Button tone="accent" icon="plus" class="bundle-view-new-btn" onClick={act.handleNew}>
                    New Bundle
                </Button>
                <Button class="bundle-view-new-btn" onClick={act.handleImportBundle}>
                    Import Bundle
                </Button>
            </div>
            <ul class="bundle-view-list">
                <For each={model.bundlesAtom()}>
                    {(memory) => (
                        <li
                            class="bundle-view-list-item"
                            classList={{
                                "is-selected": model.selectedIdAtom() === memory.id,
                                "is-blank": !!memory.is_blank,
                                "is-global": !!memory.is_global,
                            }}
                            onClick={() => act.handleSelect(memory)}
                        >
                            <div class="bundle-view-list-item-name">
                                <span class="bundle-view-list-item-name-text">
                                    {memory.is_blank ? "— Blank (vanilla CLI) —" : memory.name}
                                </span>
                                <Show when={memory.is_global}>
                                    <Tag size="xs" tone="accent" detail="Injected into all agents at launch">Global</Tag>
                                </Show>
                            </div>
                            {/* Subtitle shows the description now that bundles
                                are provider-agnostic (§4.1a). Class name kept
                                to avoid CSS churn. */}
                            <Show when={!memory.is_blank && memory.description}>
                                <div class="bundle-view-list-item-provider">
                                    {memory.description}
                                </div>
                            </Show>
                        </li>
                    )}
                </For>
            </ul>
        </div>
    );
}
