// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { Show } from "solid-js";
import type { JSX } from "solid-js";
import type { ForgeViewModel } from "./forge-model";
import { ForgeList } from "./components/ForgeList";
import { ForgeDetail } from "./components/ForgeDetail";
import { ForgeForm } from "./components/ForgeForm";
import "./forge-view.scss";

export function ForgeView(props: ViewComponentProps<ForgeViewModel>): JSX.Element {
    const view = props.model.viewAtom;

    return (
        <Show when={view() === "create" || view() === "edit"} fallback={
            <Show when={view() === "detail"} fallback={
                <ForgeList model={props.model} />
            }>
                <ForgeDetail model={props.model} />
            </Show>
        }>
            <ForgeForm model={props.model} />
        </Show>
    );
}
