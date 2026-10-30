// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The tooltip body of a card's Free or Local tag: the lines the access data
 * gives (the lead first, at most four) and the muted "checked <date>" last, in
 * the warning colour once the claims are stale. A tag's tooltip holds no link
 * (it closes when the pointer leaves), so the sign-up link lives in the card's
 * panel (`FreeStartGuide`).
 * docs/specs/SPEC_PROVIDER_CARD_TAGS_FREE_LOCAL_2026_10_29.md §3 and §5.
 */

import { For, type JSX } from "solid-js";
import type { AccessTag } from "../defaults/provider-access-model";
import "./AccessTagDetail.scss";

export const AccessTagDetail = (props: { tag: AccessTag }): JSX.Element => (
    <div class="agent-access-tag-detail" data-testid={`agent-access-tag-detail-${props.tag.label.toLowerCase()}`}>
        <For each={props.tag.lines}>
            {(line, i) => <div classList={{ "agent-access-tag-detail-lead": i() === 0 }}>{line}</div>}
        </For>
        <div
            class="agent-access-tag-detail-checked"
            classList={{ "agent-access-tag-detail-checked--stale": props.tag.checked.state !== "fresh" }}
        >
            {props.tag.checked.text}
        </div>
    </div>
);
