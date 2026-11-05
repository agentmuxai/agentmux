// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A bundle's read-only view: its name and scope, Edit and Delete, then what it holds as cards by category. */

import { Show, type Accessor, type JSX } from "solid-js";
import { Button, Card, CardGrid, Tag } from "@/app/element/ui";
import type { Bundle } from "@/app/store/rpc-api";
import type { BundleActions } from "./bundle-manager-actions";
import type { BundleViewModel } from "./bundle-model";
import { BundleCategory } from "./bundle-category";
import { BundleSkillsSection } from "./BundleSkillsSection";

/** How many harness-scoped instruction variants a bundle carries (the stored JSON object's keys). */
function variantCount(bundle: Bundle): number {
    try {
        return Object.keys(JSON.parse(bundle.instructions_by_provider || "{}")).length;
    } catch {
        return 0;
    }
}

/** The bundle's instructions as one card; opening it opens the edit form, where they are written. */
function InstructionsCard(props: { bundle: Bundle; onEdit?: () => void }): JSX.Element {
    const text = () => (props.bundle.instructions || "").trim();
    const variants = () => variantCount(props.bundle);
    return (
        <Card
            class="bundle-instructions-card"
            icon={<i class="fa-solid fa-file-lines" />}
            title="Instructions"
            subtitle={text().split("\n")[0] || "(none)"}
            meta={[
                <span>{text().length} characters</span>,
                ...(variants() > 0 ? [<span>{variants()} provider {variants() === 1 ? "variant" : "variants"}</span>] : []),
            ]}
            onSelect={props.onEdit}
        />
    );
}

export function BundleReadOnly(props: {
    model: BundleViewModel;
    act: BundleActions;
    memory: Accessor<Bundle>;
}): JSX.Element {
    const { model, act, memory } = props;
    return (
        <div class="bundle-view-readonly ui-density-compact">
            <h2 class="bundle-view-name">{memory().name}</h2>
            <Show when={memory().description}>
                <p class="bundle-view-description">{memory().description}</p>
            </Show>
            <Show when={memory().is_global}>
                <p class="bundle-view-scope">
                    <Tag size="xs" tone="accent" detail="Injected into all agents at launch">Global</Tag>
                    {" "}— injected into every agent at launch
                </p>
            </Show>
            <Show when={!memory().is_blank}>
                <div class="bundle-view-actions">
                    <Button density="compact" class="bundle-view-edit-btn" onClick={() => model.startEdit(memory())}>
                        Edit
                    </Button>
                    <Button
                        density="compact"
                        tone="danger"
                        class="bundle-view-delete-btn"
                        onClick={() => act.handleDelete(memory().id)}
                    >
                        Delete
                    </Button>
                </div>
            </Show>
            <BundleCategory title="Instructions">
                <CardGrid
                    ariaLabel="Instructions"
                    items={[memory()]}
                    keyOf={(bundle) => bundle.id}
                    renderCard={(bundle) => (
                        <InstructionsCard
                            bundle={bundle}
                            onEdit={bundle.is_blank ? undefined : () => model.startEdit(bundle)}
                        />
                    )}
                />
            </BundleCategory>
            {/* Skills are managed live via their own bind/unbind + upsert-for-bundle RPCs, independent of the
                edit-draft flow. A bundle carries no MCP servers; they're bound to agents in Connectors
                (SPEC_BUNDLE_CONTENTS_MEMORY_NOT_MCP_2026_10_07.md). Packs (Area54 #1560) become one more
                <BundleCategory> here once a bundle can hold them. */}
            <Show when={!memory().is_blank}>
                <BundleSkillsSection bundleId={memory().id} />
            </Show>
        </div>
    );
}
