// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BundleListEditor — pick an agent's bundles, in order: the new-agent form,
 * the launch modal and the Stash's Bundles tab
 * (SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md §3.6). Order matters: when
 * two bundles name the same MCP server or skill, the first wins.
 *
 * Controlled: `value` is the picked ids, `onChange` gets the next list. The
 * agent's own bundle, when given as `own`, is shown first and can't be moved
 * or removed; it isn't part of `value`.
 */

import { createMemo, For, Show, type JSX } from "solid-js";
import { IconButton, Select } from "@/app/element/ui";
import type { Bundle } from "@/app/store/rpc-api";
import "./BundleListEditor.scss";

export interface BundleListEditorProps {
    /** The bundles that can be picked (callers leave out blank and system ones). */
    bundles: Bundle[];
    value: string[];
    onChange: (ids: string[]) => void;
    /** The agent's own bundle's name, shown first and fixed. */
    own?: string;
    disabled?: boolean;
    testId?: string;
}

export function BundleListEditor(props: BundleListEditorProps): JSX.Element {
    const byId = createMemo(() => new Map(props.bundles.map((b) => [b.id, b])));
    const remaining = createMemo(() => props.bundles.filter((b) => !props.value.includes(b.id)));

    const move = (index: number, by: -1 | 1) => {
        const next = [...props.value];
        const [id] = next.splice(index, 1);
        next.splice(index + by, 0, id);
        props.onChange(next);
    };
    const remove = (index: number) => props.onChange(props.value.filter((_, i) => i !== index));
    const add = (id: string) => {
        if (id && !props.value.includes(id)) props.onChange([...props.value, id]);
    };

    return (
        <div class="bundle-list-editor" data-testid={props.testId}>
            <Show when={props.own}>
                {(own) => (
                    <div class="bundle-list-editor-row is-own">
                        <span class="bundle-list-editor-name">{own()}</span>
                        <span class="bundle-list-editor-hint">this agent's own</span>
                    </div>
                )}
            </Show>
            <For each={props.value}>
                {(id, index) => (
                    <div class="bundle-list-editor-row" data-testid="bundle-list-editor-item">
                        <span class="bundle-list-editor-name" classList={{ "is-missing": !byId().get(id) }}>
                            {byId().get(id)?.name ?? "Deleted bundle"}
                        </span>
                        <IconButton
                            icon="arrow-up"
                            label="Move up"
                            disabled={props.disabled || index() === 0}
                            onClick={() => move(index(), -1)}
                        />
                        <IconButton
                            icon="arrow-down"
                            label="Move down"
                            disabled={props.disabled || index() === props.value.length - 1}
                            onClick={() => move(index(), 1)}
                        />
                        <IconButton
                            icon="xmark"
                            label="Remove"
                            disabled={props.disabled}
                            onClick={() => remove(index())}
                        />
                    </div>
                )}
            </For>
            <Show
                when={remaining().length > 0}
                fallback={
                    <Show when={props.bundles.length === 0}>
                        <span class="bundle-list-editor-hint">No bundles yet. Create one in Memory → Bundles.</span>
                    </Show>
                }
            >
                <Select
                    value=""
                    onChange={add}
                    disabled={props.disabled}
                    ariaLabel="Add a bundle"
                    data-testid="bundle-list-editor-add"
                >
                    <option value="" disabled>
                        {props.value.length === 0 ? "Add a bundle…" : "Add another bundle…"}
                    </option>
                    <For each={remaining()}>{(bundle) => <option value={bundle.id}>{bundle.name}</option>}</For>
                </Select>
            </Show>
        </div>
    );
}
