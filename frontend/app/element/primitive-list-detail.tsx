// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// PrimitiveListDetail — shared single-pane list/detail layout for Armory's
// Bundles/Skills/MCP Servers tabs (and any future list+CRUD-form primitive
// with the same shape). Renders exactly ONE of {list, detail} at a time —
// never both, at any pane width. Replaces the previous fixed-width
// side-by-side split (and, for Bundles, a stacked-but-still-both-visible
// narrow-width fallback) that didn't hold up in a thin pane. See
// docs/specs/SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md.
//
// Not a tree: this is a flat two-state stack (list, or one item's detail),
// never a nested category/group drill-down.

import { Show, type JSX } from "solid-js";

import "./primitive-list-detail.scss";

export function PrimitiveListDetail(props: {
    /** True when the detail view (read-only or edit form) should show instead of the list. */
    showDetail: boolean;
    /** Label after the back chevron, e.g. "Bundles", "Skills", "MCP Servers". */
    backLabel: string;
    onBack: () => void;
    list: JSX.Element;
    detail: JSX.Element;
}): JSX.Element {
    return (
        <div class="primitive-list-detail">
            <Show when={!props.showDetail}>
                <div class="primitive-list-detail-list">{props.list}</div>
            </Show>
            <Show when={props.showDetail}>
                <div class="primitive-list-detail-detail">
                    <button
                        type="button"
                        class="primitive-list-detail-back-btn"
                        onClick={() => props.onBack()}
                    >
                        <i class="fa-sharp fa-solid fa-chevron-left" aria-hidden="true" />
                        <span>{props.backLabel}</span>
                    </button>
                    <div class="primitive-list-detail-detail-body">{props.detail}</div>
                </div>
            </Show>
        </div>
    );
}
