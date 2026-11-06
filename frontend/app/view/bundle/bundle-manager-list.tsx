// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The bundle manager's list: the error banner, New and Import, and a grid of bundle cards. */

import { Show, type JSX } from "solid-js";
import { Button, Card, CardGrid, Tag } from "@/app/element/ui";
import type { Bundle } from "@/app/store/rpc-api";
import type { BundleActions } from "./bundle-manager-actions";
import type { BundleViewModel } from "./bundle-model";

function BundleCard(props: { bundle: Bundle; act: BundleActions }): JSX.Element {
    const bundle = props.bundle;
    // The description is the card's subtitle now that bundles are provider-agnostic (§4.1a).
    const facts = () => [
        ...(bundle.is_global
            ? [
                  <Tag size="xs" tone="accent" detail="Injected into all agents at launch">
                      Global
                  </Tag>,
              ]
            : []),
        ...(bundle.provider ? [<span>{bundle.provider}</span>] : []),
    ];
    return (
        <Card
            class="bundle-card"
            icon={<i class="fa-solid fa-layer-group" />}
            title={bundle.is_blank ? "— Blank (vanilla CLI) —" : bundle.name}
            subtitle={!bundle.is_blank && bundle.description ? bundle.description : undefined}
            meta={facts()}
            muted={!!bundle.is_blank}
            onSelect={() => props.act.handleSelect(bundle)}
        />
    );
}

export function BundleList(props: { model: BundleViewModel; act: BundleActions }): JSX.Element {
    const { model, act } = props;
    return (
        <div class="bundle-view-rail ui-density-compact">
            <div class="primitive-card-actions">
                <Button tone="accent" icon="plus" density="compact" onClick={act.handleNew}>
                    New Bundle
                </Button>
                <Button density="compact" onClick={act.handleImportBundle}>
                    Import Bundle
                </Button>
            </div>
            <Show when={model.errorAtom()}>
                <div class="bundle-view-error">{model.errorAtom()}</div>
            </Show>
            <div class="primitive-card-body bundle-view-cards">
                <CardGrid
                    ariaLabel="Bundles"
                    items={model.bundlesAtom()}
                    keyOf={(bundle) => bundle.id}
                    renderCard={(bundle) => <BundleCard bundle={bundle} act={act} />}
                />
            </div>
        </div>
    );
}
