// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// BundleManager — the context-free Armory Bundle Format (ABF) management UI.
//
// This is the full list / create / edit / delete lifecycle for ABF
// bundles, context-free: it doesn't depend on an Agent-pane block,
// `nodeModel`, or any ViewModel-from-BlockRegistry context. <BundleManager/>
// owns its own block-free model; the Memory pane's Bundles section renders it.
//
// Everything here drives purely off the `bundle_*` RPCs (via
// BundleViewModel) plus the `memories:changed` MPS event, so two live
// instances stay consistent for free.

import { onCleanup, type JSX } from "solid-js";

import { PrimitiveListDetail } from "@/app/element/primitive-list-detail";
import { createBundleActions } from "./bundle-manager-actions";
import { BundleDetail } from "./bundle-manager-detail";
import { BundleList } from "./bundle-manager-list";
import { BundleViewModel } from "./bundle-model";

import "./bundle-view.scss";

interface BundleManagerBodyProps {
    model: BundleViewModel;
}

/**
 * BundleManagerBody — the rail + detail UI, driven by a BundleViewModel.
 *
 * This component is context-free: it reads and writes ONLY through the
 * `model` accessors/methods, none of which require a block. Both the
 * settings-pane wrapper and the standalone <BundleManager/> render this
 * with their respective models, so the markup lives in exactly one place.
 */
const BundleManagerBody = (props: BundleManagerBodyProps): JSX.Element => {
    const { model } = props;
    const act = createBundleActions(model);

    const listView = <BundleList model={model} act={act} />;
    const detailView = <BundleDetail model={model} act={act} />;

    return (
        <PrimitiveListDetail
            showDetail={act.inDetail()}
            backLabel="Bundles"
            onBack={act.handleBack}
            list={listView}
            detail={detailView}
        />
    );
};

/**
 * BundleManager — context-free standalone bundle manager.
 *
 * Constructs its OWN block-free BundleViewModel and renders the shared
 * body. Use this wherever Memory CRUD is needed outside an Agent-pane
 * block — e.g. the window-scoped bundle manager modal. Takes no props.
 *
 * The model's only block dependency was the cosmetic header title; with
 * the optional-constructor change in bundle-model.ts it constructs
 * cleanly with no block and drives entirely off the `bundle_*` RPCs.
 */
export const BundleManager = (): JSX.Element => {
    // Component setup runs once; the model lives for this component's
    // lifetime and is disposed on unmount.
    const model = new BundleViewModel();
    onCleanup(() => model.dispose());
    return <BundleManagerBody model={model} />;
};
