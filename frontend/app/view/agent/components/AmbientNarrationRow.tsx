// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Renders ambient narrations — short lines about something AgentMux did on its
 * own, in the model's voice but visibly attributed.
 *
 * ON THE ATTRIBUTION TAG. The request was for these to "appear like they are
 * coming from the model". They are: same voice, same place in the flow, no
 * separate panel. What they are not is *indistinguishable* from the model,
 * because the model did not write them and its transcript does not contain
 * them. Read back an hour later — by a user, or by an agent debugging its own
 * conversation — an unmarked line is something the model appears to have
 * claimed, with nothing to say otherwise.
 *
 * The tag costs one short word. Removing it buys a slightly smoother read and
 * a standing, unfalsifiable claim in the transcript. If that trade is wanted,
 * delete the `<span class="ambient-narration-row-tag">` below and nothing else
 * changes — but make it a decision, not a default.
 */

import { For, Show, type JSX } from "solid-js";

import type { AmbientNarration } from "../hooks/useAmbientNarration";
import "./AmbientNarrationRow.scss";

export interface AmbientNarrationRowProps {
    narrations: AmbientNarration[];
}

export const AmbientNarrationRow = (props: AmbientNarrationRowProps): JSX.Element => (
    <Show when={props.narrations.length > 0}>
        <div class="ambient-narration-rows">
            <For each={props.narrations}>
                {(n) => (
                    <div class="ambient-narration-row" data-kind={n.kind}>
                        <span class="ambient-narration-row-tag">ambient</span>
                        <span class="ambient-narration-row-text">{n.text}</span>
                    </div>
                )}
            </For>
        </div>
    </Show>
);
