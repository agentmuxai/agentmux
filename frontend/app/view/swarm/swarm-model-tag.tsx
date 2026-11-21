// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm row's model tag: the model the agent is running now, as a short label in its family's colour, on a line
 * of its own between the header and the summary (docs/specs/SPEC_SWARM_MODEL_AND_SEAT_TAGS_2026_11_20.md §3.1 and §4).
 * It is drawn only for an agent whose model is known; nothing is guessed, and there is no placeholder.
 */

import { createMemo, For, Show, type Accessor, type JSX } from "solid-js";

import { modelTag, type ModelFacts } from "@/app/agent-model/model-tag";
import { Tag } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";

import "./swarm-tags.scss";

/** The tag for a model, or nothing. A screen reader hears "Model Sonnet 5.5"; the tooltip has the rest. */
export function SwarmModelTag(props: { facts: Accessor<ModelFacts | null | undefined> }): JSX.Element {
    const tag = createMemo(() => {
        const facts = props.facts();
        return facts ? modelTag(facts) : null;
    });
    return (
        <Show when={tag()}>
            {(shown) => (
                <Tag
                    hue={shown().hue}
                    muted={shown().muted}
                    icon={shown().glyph}
                    class="swarm-model-tag"
                    detail={<For each={shown().lines}>{(line) => <div>{line}</div>}</For>}
                >
                    <span class="ui-tag-sr">{t("swarm.tag.model")} </span>
                    <span class="swarm-model-tag__text">{shown().text}</span>
                </Tag>
            )}
        </Show>
    );
}

/** The line under a row's header that holds the model tag; no line at all when the model is not known. */
export function SwarmTagsLine(props: { facts: Accessor<ModelFacts | null | undefined> }): JSX.Element {
    const known = () => {
        const facts = props.facts();
        return facts != null && modelTag(facts) != null;
    };
    return (
        <Show when={known()}>
            <div class="swarm-tags-line">
                <SwarmModelTag facts={props.facts} />
            </div>
        </Show>
    );
}
