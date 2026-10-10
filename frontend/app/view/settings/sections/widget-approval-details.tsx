// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What the user is approving when a widget asks to be installed
// (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §8.3): its
// version, author and kind, and every permission in plain words, or the
// trusted-widget warning. Shown in Settings → Widgets and in the prompt an
// agent's install request opens, so both say exactly the same.

import { For, Show, type JSX } from "solid-js";

import type { WidgetKind } from "@/app/store/rpc-api/widgets";
import { describePermission, TRUSTED_WIDGET_WARNING } from "./widget-permissions";

import "./widget-approval.scss";

export interface WidgetApprovalFacts {
    version: string;
    author?: string | null;
    kind: WidgetKind;
    implied?: boolean;
    description?: string | null;
    permissions: string[];
}

export function WidgetApprovalDetails(props: { pkg: WidgetApprovalFacts }): JSX.Element {
    return (
        <div class="widget-approval-details">
            <div class="widget-approval-meta">
                {props.pkg.version}
                <Show when={props.pkg.author}> · by {props.pkg.author}</Show>
                {" · "}
                {props.pkg.kind === "trusted" ? "trusted" : "sandboxed"}
                <Show when={props.pkg.implied}> · from widgets.json</Show>
            </div>
            <Show when={props.pkg.description}>
                <div class="widget-approval-description">{props.pkg.description}</div>
            </Show>
            <Show
                when={props.pkg.kind === "sandboxed"}
                fallback={
                    <div class="widget-approval-warning">
                        <i class="fa-solid fa-triangle-exclamation" /> <strong>{TRUSTED_WIDGET_WARNING}</strong>
                    </div>
                }
            >
                <div class="widget-approval-permissions">
                    <Show when={props.pkg.permissions.length > 0} fallback={<div>It asks for no permissions.</div>}>
                        <div>It can:</div>
                        <ul>
                            <For each={props.pkg.permissions}>
                                {(p) => {
                                    const d = describePermission(p);
                                    return (
                                        <li classList={{ strong: !!d.strong }}>
                                            <Show when={d.strong}>
                                                <i class="fa-solid fa-triangle-exclamation" />{" "}
                                            </Show>
                                            {d.text}
                                        </li>
                                    );
                                }}
                            </For>
                        </ul>
                    </Show>
                </div>
            </Show>
        </div>
    );
}
